//! Normalized contract manifest types for generated SDKs, OpenAPI, validation metadata, and realtime contracts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::foundation::{Error, Result};
use crate::http::RouteManifestEntry;

pub const CONTRACT_MANIFEST_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ContractManifest {
    pub version: u32,
    pub schemas: Vec<ContractSchema>,
    pub validation_schemas: Vec<ContractValidationSchema>,
    pub actions: Vec<ContractAction>,
    pub realtime_channels: Vec<ContractRealtimeChannel>,
    pub errors: Vec<ContractError>,
}

impl ContractManifest {
    pub fn new() -> Self {
        Self {
            version: CONTRACT_MANIFEST_VERSION,
            errors: ContractError::standard_errors(),
            ..Self::default()
        }
    }

    pub fn from_http_routes(routes: &[RouteManifestEntry]) -> Result<Self> {
        let mut manifest = Self::new();
        manifest.actions = routes
            .iter()
            .map(ContractAction::from_http_route)
            .collect::<Result<Vec<_>>>()?;
        Ok(manifest)
    }

    pub fn with_schemas(mut self, schemas: Vec<ContractSchema>) -> Self {
        self.schemas = schemas;
        self
    }

    pub fn with_validation_schemas(mut self, schemas: Vec<ContractValidationSchema>) -> Self {
        self.validation_schemas = schemas;
        self.infer_transport_body_kinds();
        self
    }

    pub fn with_realtime_channels(mut self, channels: Vec<ContractRealtimeChannel>) -> Self {
        self.realtime_channels = channels;
        self
    }

    pub fn infer_transport_body_kinds(&mut self) {
        let validation_by_name = self
            .validation_schemas
            .iter()
            .map(|schema| (schema.name.as_str(), schema))
            .collect::<BTreeMap<_, _>>();

        for action in &mut self.actions {
            let Some(request) = action.request.as_ref() else {
                continue;
            };
            let Some(schema) = validation_by_name.get(request.schema.as_str()) else {
                continue;
            };

            if schema.requires_multipart() {
                if let ContractTransport::Http(http) = &mut action.transport {
                    http.body = ContractHttpBody::Multipart;
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContractSchema {
    pub name: String,
    pub schema: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContractAction {
    pub id: String,
    pub action_name: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub deprecated: bool,
    pub request: Option<ContractPayload>,
    pub responses: Vec<ContractResponse>,
    pub auth: ContractAuth,
    pub client_export: bool,
    pub validation: Option<String>,
    pub transport: ContractTransport,
}

impl ContractAction {
    pub fn from_http_route(route: &RouteManifestEntry) -> Result<Self> {
        let id = route.id.as_str().to_string();
        let request = route.request.map(|schema| ContractPayload {
            schema: schema.to_string(),
        });
        let body = match (&route.method, request.as_ref()) {
            (_, None) => ContractHttpBody::None,
            (Some(method), Some(_))
                if matches!(method.as_str(), "post" | "put" | "patch" | "delete") =>
            {
                ContractHttpBody::Json
            }
            (Some(_), Some(_)) => ContractHttpBody::None,
            (None, Some(_)) => ContractHttpBody::Unknown,
        };

        Ok(Self {
            id: id.clone(),
            action_name: to_pascal_case_identifier(&id, "contract action name")?,
            summary: route.summary.clone(),
            description: None,
            tags: Vec::new(),
            deprecated: false,
            validation: request.as_ref().map(|payload| payload.schema.clone()),
            request,
            responses: route
                .responses
                .iter()
                .map(|response| ContractResponse {
                    status: response.status,
                    schema: response.schema.to_string(),
                    schema_json: response.schema_json.clone(),
                })
                .collect(),
            auth: ContractAuth {
                guard: route.guard.as_ref().map(|guard| guard.as_str().to_string()),
                permissions: route
                    .permissions
                    .iter()
                    .map(|permission| permission.as_str().to_string())
                    .collect(),
            },
            client_export: route.client_export,
            transport: ContractTransport::Http(ContractHttpTransport {
                method: route.method.clone(),
                path: route.path.clone(),
                path_params: route.params.clone(),
                body,
            }),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractPayload {
    pub schema: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContractResponse {
    pub status: u16,
    pub schema: String,
    #[serde(skip)]
    pub(crate) schema_json: Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractAuth {
    pub guard: Option<String>,
    pub permissions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractError {
    pub code: String,
    pub status: u16,
    pub schema: Option<String>,
}

impl ContractError {
    pub fn standard_errors() -> Vec<Self> {
        [
            ("validation_failed", 422),
            ("unauthorized", 401),
            ("forbidden", 403),
            ("not_found", 404),
            ("internal_error", 500),
        ]
        .into_iter()
        .map(|(code, status)| Self {
            code: code.to_string(),
            status,
            schema: None,
        })
        .collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContractTransport {
    Http(ContractHttpTransport),
    WebSocket(ContractWebSocketTransport),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractHttpTransport {
    pub method: Option<String>,
    pub path: String,
    pub path_params: Vec<String>,
    pub body: ContractHttpBody,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContractHttpBody {
    None,
    Json,
    Multipart,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractWebSocketTransport {
    pub channel: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractValidationSchema {
    pub name: String,
    pub fields: Vec<ContractValidationField>,
    pub messages: Vec<ContractValidationMessage>,
    pub attributes: Vec<ContractValidationAttribute>,
}

impl ContractValidationSchema {
    pub fn requires_multipart(&self) -> bool {
        self.fields.iter().any(|field| {
            matches!(
                field.value_kind,
                ContractValueKind::File | ContractValueKind::FileList
            ) || field.rules.iter().any(ContractValidationRule::is_file_rule)
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractValidationField {
    pub name: String,
    pub value_kind: ContractValueKind,
    pub rules: Vec<ContractValidationRule>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractValidationRule {
    pub code: String,
    pub params: BTreeMap<String, String>,
    pub values: Vec<String>,
    pub message: Option<String>,
    pub server_only: bool,
    pub rules: Vec<ContractValidationRule>,
}

impl ContractValidationRule {
    pub fn is_file_rule(&self) -> bool {
        matches!(
            self.code.as_str(),
            "image"
                | "max_file_size"
                | "max_dimensions"
                | "min_dimensions"
                | "allowed_mimes"
                | "allowed_extensions"
        ) || self.rules.iter().any(Self::is_file_rule)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractValidationMessage {
    pub field: String,
    pub rule: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractValidationAttribute {
    pub field: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContractValueKind {
    #[default]
    Unknown,
    Scalar,
    Array,
    Object,
    File,
    FileList,
    Date,
    DateTime,
    LocalDateTime,
    Time,
    Decimal,
    Uuid,
    Json,
    Page,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractRealtimeChannel {
    pub id: String,
    pub presence: bool,
    pub replay_count: u32,
    pub allow_client_events: bool,
    pub auth: ContractAuth,
    pub incoming: Vec<ContractRealtimeEvent>,
    pub outgoing: Vec<ContractRealtimeEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractRealtimeEvent {
    pub event: String,
    pub payload: Option<ContractPayload>,
}

impl From<&crate::websocket::WebSocketChannelDescriptor> for ContractRealtimeChannel {
    fn from(channel: &crate::websocket::WebSocketChannelDescriptor) -> Self {
        Self {
            id: channel.id.as_str().to_string(),
            presence: channel.presence,
            replay_count: channel.replay_count,
            allow_client_events: channel.allow_client_events,
            auth: ContractAuth {
                guard: channel
                    .guard
                    .as_ref()
                    .map(|guard| guard.as_str().to_string()),
                permissions: channel
                    .permissions
                    .iter()
                    .map(|permission| permission.as_str().to_string())
                    .collect(),
            },
            incoming: channel
                .incoming
                .iter()
                .map(|event| ContractRealtimeEvent {
                    event: event.event.as_str().to_string(),
                    payload: event.payload.as_ref().map(|schema| ContractPayload {
                        schema: schema.clone(),
                    }),
                })
                .collect(),
            outgoing: channel
                .outgoing
                .iter()
                .map(|event| ContractRealtimeEvent {
                    event: event.event.as_str().to_string(),
                    payload: event.payload.as_ref().map(|schema| ContractPayload {
                        schema: schema.clone(),
                    }),
                })
                .collect(),
        }
    }
}

fn is_ts_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if !(first == '_' || first == '$' || first.is_ascii_alphabetic()) {
        return false;
    }

    chars.all(|ch| ch == '_' || ch == '$' || ch.is_ascii_alphanumeric())
}

fn to_pascal_case_identifier(value: &str, context: &str) -> Result<String> {
    let mut words = Vec::new();
    let mut current = String::new();

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch);
        } else if ch.is_ascii() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else {
            return Err(Error::message(format!(
                "{context} only supports ASCII identifiers; `{value}` contains unsupported character `{ch}`"
            )));
        }
    }

    if !current.is_empty() {
        words.push(current);
    }

    if words.is_empty() {
        return Err(Error::message(format!(
            "{context} requires a non-empty identifier"
        )));
    }

    let mut identifier = String::new();
    for word in words {
        let lower = word.to_ascii_lowercase();
        let mut chars = lower.chars();
        if let Some(first) = chars.next() {
            identifier.push(first.to_ascii_uppercase());
            identifier.push_str(chars.as_str());
        }
    }

    if !is_ts_identifier(&identifier) {
        return Err(Error::message(format!(
            "{context} normalized `{value}` to invalid identifier `{identifier}`"
        )));
    }

    Ok(identifier)
}
