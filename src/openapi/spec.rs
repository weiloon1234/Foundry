use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::contract::{
    ContractAction, ContractAuth, ContractError, ContractHttpBody, ContractHttpTransport,
    ContractManifest, ContractParameter, ContractParameterLocation, ContractPayload,
    ContractResponse, ContractSchema, ContractTransport,
};
use crate::http::route_path_params;
use crate::http::routes::route_segment_param;

use super::{ApiSchema, RouteDoc};

pub struct DocumentedRoute {
    pub method: String,
    pub path: String,
    pub doc: RouteDoc,
    pub auth: ContractAuth,
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
    let has_guarded_actions = manifest
        .actions
        .iter()
        .any(|action| action.auth.guard.is_some());
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
        let mut operation = json!({
            "operationId": action.action_name,
        });
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
            operation["security"] = json!([{ "bearerAuth": [] }]);
        }
        if !action.auth.permissions.is_empty() {
            operation["x-foundry-permissions"] = json!(action.auth.permissions);
        }

        if !action.parameters.is_empty() {
            operation["parameters"] = Value::Array(
                action
                    .parameters
                    .iter()
                    .map(|parameter| {
                        let location = match parameter.location {
                            ContractParameterLocation::Path => "path",
                            ContractParameterLocation::Query => "query",
                            ContractParameterLocation::Header => "header",
                            ContractParameterLocation::Cookie => "cookie",
                        };
                        json!({
                            "name": parameter.name,
                            "in": location,
                            "required": parameter.required,
                            "schema": {
                                "$ref": format!("#/components/schemas/{}", parameter.schema)
                            }
                        })
                    })
                    .collect(),
            );
        }

        if let Some(req) = action
            .request
            .as_ref()
            .filter(|_| http.body != ContractHttpBody::None || http.content_type.is_some())
        {
            let content_type = http.content_type.as_deref().unwrap_or(match http.body {
                ContractHttpBody::Multipart => "multipart/form-data",
                ContractHttpBody::Json | ContractHttpBody::Unknown => "application/json",
                ContractHttpBody::None => "application/octet-stream",
            });
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

        operation["responses"] = openapi_responses(manifest, action);

        let path_entry = paths
            .entry(openapi_path(&http.path))
            .or_insert_with(|| json!({}));
        path_entry[&method] = operation;
    }

    let security_schemes = if has_guarded_actions {
        json!({
            "bearerAuth": {
                "type": "http",
                "scheme": "bearer",
                "bearerFormat": "Foundry token"
            }
        })
    } else {
        json!({})
    };

    json!({
        "openapi": "3.1.0",
        "info": { "title": title, "version": version },
        "paths": paths,
        "components": {
            "schemas": schemas,
            "securitySchemes": security_schemes
        }
    })
}

fn openapi_responses(manifest: &ContractManifest, action: &ContractAction) -> Value {
    let mut responses = serde_json::Map::new();

    for response in &action.responses {
        responses.insert(
            response.status.to_string(),
            json!({
                "description": status_description(response.status),
                "content": {
                    "application/json": {
                        "schema": {
                            "$ref": format!("#/components/schemas/{}", response.schema)
                        }
                    }
                }
            }),
        );
    }

    let mut errors_by_status = BTreeMap::<u16, Vec<&ContractError>>::new();
    for error in manifest.errors.iter().chain(&action.errors) {
        errors_by_status
            .entry(error.status)
            .or_default()
            .push(error);
    }
    for (status, errors) in errors_by_status {
        responses.insert(status.to_string(), openapi_error_response(status, &errors));
    }

    if responses.is_empty() {
        responses.insert("default".to_string(), json!({ "description": "Response" }));
    }

    Value::Object(responses)
}

fn openapi_error_response(status: u16, errors: &[&ContractError]) -> Value {
    let mut codes = errors
        .iter()
        .map(|error| error.code.as_str())
        .collect::<Vec<_>>();
    codes.sort_unstable();
    codes.dedup();
    let mut response = json!({
        "description": format!("{} ({})", status_description(status), codes.join(", ")),
        "x-foundry-error-codes": codes,
    });

    let mut schemas = errors
        .iter()
        .filter_map(|error| error.schema.as_deref())
        .collect::<Vec<_>>();
    schemas.sort_unstable();
    schemas.dedup();
    if !schemas.is_empty() {
        let schema = if schemas.len() == 1 {
            json!({ "$ref": format!("#/components/schemas/{}", schemas[0]) })
        } else {
            json!({
                "oneOf": schemas
                    .iter()
                    .map(|schema| json!({ "$ref": format!("#/components/schemas/{schema}") }))
                    .collect::<Vec<_>>()
            })
        };
        response["content"] = json!({
            "application/json": {
                "schema": schema
            }
        });
    }

    response
}

fn status_description(status: u16) -> String {
    axum::http::StatusCode::from_u16(status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("HTTP {status} response"))
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

        let mut parameters = route
            .doc
            .parameters
            .iter()
            .map(|parameter| {
                schema_map.insert(
                    parameter.schema.name.to_string(),
                    (parameter.schema.schema_fn)(),
                );
                ContractParameter {
                    name: parameter.name.clone(),
                    location: parameter.location,
                    schema: parameter.schema.name.to_string(),
                    required: parameter.required,
                }
            })
            .collect::<Vec<_>>();
        for name in route_path_params(&route.path) {
            if parameters.iter().any(|parameter| {
                parameter.location == ContractParameterLocation::Path && parameter.name == name
            }) {
                continue;
            }
            schema_map.insert(String::schema_name().to_string(), String::schema());
            parameters.push(ContractParameter {
                name,
                location: ContractParameterLocation::Path,
                schema: String::schema_name().to_string(),
                required: true,
            });
        }

        let errors = route
            .doc
            .errors
            .iter()
            .map(|error| {
                let schema = error.schema.as_ref().map(|schema| {
                    schema_map.insert(schema.name.to_string(), (schema.schema_fn)());
                    schema.name.to_string()
                });
                ContractError {
                    code: error.code.clone(),
                    status: error.status,
                    schema,
                }
            })
            .collect::<Vec<_>>();

        actions.push(ContractAction {
            id: format!("{} {}", route.method, route.path),
            action_name: route
                .doc
                .action_name
                .clone()
                .unwrap_or_else(|| format!("OpenApiRoute{}", index + 1)),
            summary: route.doc.summary.clone(),
            description: route.doc.description.clone(),
            tags: route.doc.tags.clone(),
            deprecated: route.doc.deprecated,
            request,
            parameters,
            responses,
            errors,
            auth: route.auth.clone(),
            client_export: false,
            validation: None,
            transport: ContractTransport::Http(ContractHttpTransport {
                method: Some(route.method.clone()),
                path: route.path.clone(),
                body: if route.doc.request.is_some() {
                    ContractHttpBody::Json
                } else {
                    ContractHttpBody::None
                },
                content_type: route.doc.request_content_type.clone(),
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

fn openapi_path(path: &str) -> String {
    let mut normalized = String::with_capacity(path.len());
    for (index, segment) in path.split('/').enumerate() {
        if index > 0 {
            normalized.push('/');
        }

        if let Some(param) = route_segment_param(segment) {
            normalized.push('{');
            normalized.push_str(param);
            normalized.push('}');
        } else {
            normalized.push_str(segment);
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::openapi::ApiSchema;

    struct UserSchema;

    impl ApiSchema for UserSchema {
        fn schema() -> Value {
            json!({"type": "object", "properties": {"user_name": {"type": "string"}}})
        }

        fn schema_name() -> &'static str {
            "User"
        }
    }

    struct OrderSchema;

    impl ApiSchema for OrderSchema {
        fn schema() -> Value {
            json!({"type": "object", "properties": {"order_number": {"type": "string"}}})
        }

        fn schema_name() -> &'static str {
            "Order"
        }
    }

    #[test]
    fn bodyless_contract_request_does_not_emit_request_body() {
        let mut manifest = ContractManifest::new().with_schemas(vec![
            ContractSchema {
                name: "ShowUserRequest".to_string(),
                schema: json!({ "type": "object" }),
            },
            ContractSchema {
                name: "String".to_string(),
                schema: json!({ "type": "string" }),
            },
        ]);
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
            parameters: vec![ContractParameter {
                name: "id".to_string(),
                location: ContractParameterLocation::Path,
                schema: "String".to_string(),
                required: true,
            }],
            responses: Vec::new(),
            errors: Vec::new(),
            auth: ContractAuth::default(),
            client_export: true,
            validation: Some("ShowUserRequest".to_string()),
            transport: ContractTransport::Http(ContractHttpTransport {
                method: Some("get".to_string()),
                path: "/users/{id}".to_string(),
                body: ContractHttpBody::None,
                content_type: None,
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

    #[test]
    fn openapi_paths_use_braced_parameter_syntax() {
        let mut manifest = ContractManifest::new().with_schemas(vec![ContractSchema {
            name: "String".to_string(),
            schema: json!({ "type": "string" }),
        }]);
        manifest.actions = vec![ContractAction {
            id: "files.show".to_string(),
            action_name: "FilesShow".to_string(),
            summary: None,
            description: None,
            tags: Vec::new(),
            deprecated: false,
            request: None,
            parameters: vec![
                ContractParameter {
                    name: "id".to_string(),
                    location: ContractParameterLocation::Path,
                    schema: "String".to_string(),
                    required: true,
                },
                ContractParameter {
                    name: "path".to_string(),
                    location: ContractParameterLocation::Path,
                    schema: "String".to_string(),
                    required: true,
                },
            ],
            responses: Vec::new(),
            errors: Vec::new(),
            auth: ContractAuth::default(),
            client_export: true,
            validation: None,
            transport: ContractTransport::Http(ContractHttpTransport {
                method: Some("get".to_string()),
                path: "/users/:id/files/{*path}".to_string(),
                body: ContractHttpBody::None,
                content_type: None,
            }),
        }];

        let spec = generate_openapi_spec_from_contract("Foundry", "test", &manifest);

        assert!(spec["paths"].get("/users/{id}/files/{path}").is_some());
        assert!(spec["paths"].get("/users/:id/files/{*path}").is_none());
        assert_eq!(
            spec["paths"]["/users/{id}/files/{path}"]["get"]["parameters"],
            json!([
                {
                    "name": "id",
                    "in": "path",
                    "required": true,
                    "schema": {"$ref": "#/components/schemas/String"}
                },
                {
                    "name": "path",
                    "in": "path",
                    "required": true,
                    "schema": {"$ref": "#/components/schemas/String"}
                }
            ])
        );
    }

    #[test]
    fn contract_openapi_emits_operation_security_parameters_and_standard_errors() {
        let mut manifest = ContractManifest::new().with_schemas(vec![
            ContractSchema {
                name: "String".to_string(),
                schema: String::schema(),
            },
            ContractSchema {
                name: "bool".to_string(),
                schema: bool::schema(),
            },
            ContractSchema {
                name: "i64".to_string(),
                schema: i64::schema(),
            },
            ContractSchema {
                name: "User".to_string(),
                schema: UserSchema::schema(),
            },
        ]);
        manifest.actions = vec![ContractAction {
            id: "admin.users.show".to_string(),
            action_name: "GetAdminUser".to_string(),
            summary: Some("Get an admin user".to_string()),
            description: None,
            tags: vec!["admin:users".to_string()],
            deprecated: false,
            request: None,
            parameters: vec![
                ContractParameter {
                    name: "id".to_string(),
                    location: ContractParameterLocation::Path,
                    schema: "i64".to_string(),
                    required: true,
                },
                ContractParameter {
                    name: "include_deleted".to_string(),
                    location: ContractParameterLocation::Query,
                    schema: "bool".to_string(),
                    required: false,
                },
                ContractParameter {
                    name: "x-tenant".to_string(),
                    location: ContractParameterLocation::Header,
                    schema: "String".to_string(),
                    required: true,
                },
                ContractParameter {
                    name: "locale".to_string(),
                    location: ContractParameterLocation::Cookie,
                    schema: "String".to_string(),
                    required: false,
                },
            ],
            responses: vec![ContractResponse {
                status: 200,
                schema: "User".to_string(),
                schema_json: UserSchema::schema(),
            }],
            errors: vec![ContractError {
                code: "account_missing".to_string(),
                status: 404,
                schema: Some("User".to_string()),
            }],
            auth: ContractAuth {
                guard: Some("admin".to_string()),
                permissions: vec!["users.read".to_string()],
            },
            client_export: true,
            validation: None,
            transport: ContractTransport::Http(ContractHttpTransport {
                method: Some("get".to_string()),
                path: "/admin/users/{id}".to_string(),
                body: ContractHttpBody::None,
                content_type: None,
            }),
        }];

        let spec = generate_openapi_spec_from_contract("Foundry", "test", &manifest);
        let operation = &spec["paths"]["/admin/users/{id}"]["get"];

        assert_eq!(operation["operationId"], "GetAdminUser");
        assert_eq!(operation["security"], json!([{ "bearerAuth": [] }]));
        assert_eq!(operation["x-foundry-guard"], "admin");
        assert_eq!(operation["x-foundry-permissions"], json!(["users.read"]));
        assert_eq!(
            spec["components"]["securitySchemes"]["bearerAuth"]["type"],
            "http"
        );
        assert_eq!(
            spec["components"]["securitySchemes"]["bearerAuth"]["scheme"],
            "bearer"
        );

        for (name, location, required, schema) in [
            ("id", "path", true, "i64"),
            ("include_deleted", "query", false, "bool"),
            ("x-tenant", "header", true, "String"),
            ("locale", "cookie", false, "String"),
        ] {
            assert!(operation["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .any(|parameter| {
                    parameter["name"] == name
                        && parameter["in"] == location
                        && parameter["required"] == required
                        && parameter["schema"]["$ref"] == format!("#/components/schemas/{schema}")
                }));
        }

        assert_eq!(operation["responses"]["200"]["description"], "OK");
        assert_eq!(
            operation["responses"]["404"]["x-foundry-error-codes"],
            json!(["account_missing", "not_found"])
        );
        assert_eq!(
            operation["responses"]["404"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/User"
        );
        assert_eq!(
            operation["responses"]["401"]["x-foundry-error-codes"],
            json!(["unauthorized"])
        );
        assert!(operation["responses"]["500"]["description"]
            .as_str()
            .unwrap()
            .contains("Internal Server Error"));
    }

    #[test]
    fn structural_wrapper_schemas_have_unique_resolvable_names() {
        let routes = vec![
            DocumentedRoute {
                method: "get".to_string(),
                path: "/users".to_string(),
                doc: RouteDoc::new().response::<Vec<UserSchema>>(200),
                auth: ContractAuth::default(),
            },
            DocumentedRoute {
                method: "get".to_string(),
                path: "/orders".to_string(),
                doc: RouteDoc::new().response::<Vec<OrderSchema>>(200),
                auth: ContractAuth::default(),
            },
        ];

        let spec = generate_openapi_spec("Foundry", "test", &routes);

        assert_eq!(
            spec["paths"]["/users"]["get"]["responses"]["200"]["content"]["application/json"]
                ["schema"]["$ref"],
            "#/components/schemas/ArrayOfUser"
        );
        assert_eq!(
            spec["paths"]["/orders"]["get"]["responses"]["200"]["content"]["application/json"]
                ["schema"]["$ref"],
            "#/components/schemas/ArrayOfOrder"
        );
        assert_eq!(
            spec["components"]["schemas"]["ArrayOfUser"]["items"],
            UserSchema::schema()
        );
        assert_eq!(
            spec["components"]["schemas"]["ArrayOfOrder"]["items"],
            OrderSchema::schema()
        );
    }

    #[test]
    fn documented_routes_can_override_request_media_type() {
        let routes = vec![DocumentedRoute {
            method: "post".to_string(),
            path: "/sessions".to_string(),
            doc: RouteDoc::new()
                .action_name("CreateSession")
                .request::<UserSchema>()
                .request_content_type("application/x-www-form-urlencoded")
                .response::<UserSchema>(201),
            auth: ContractAuth::default(),
        }];

        let spec = generate_openapi_spec("Foundry", "test", &routes);
        let operation = &spec["paths"]["/sessions"]["post"];

        assert_eq!(operation["operationId"], "CreateSession");
        assert_eq!(
            operation["requestBody"]["content"]["application/x-www-form-urlencoded"]["schema"]
                ["$ref"],
            "#/components/schemas/User"
        );
        assert_eq!(operation["responses"]["201"]["description"], "Created");
    }

    #[test]
    fn nullable_schema_uses_openapi_31_json_schema_union() {
        let schema = <Option<String> as ApiSchema>::schema();

        assert_eq!(
            schema,
            json!({"anyOf": [{"type": "string"}, {"type": "null"}]})
        );
        assert!(schema.get("nullable").is_none());
    }
}
