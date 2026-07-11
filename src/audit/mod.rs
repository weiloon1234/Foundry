use std::collections::BTreeSet;
use std::future::Future;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::auth::Actor;
use crate::database::{DbRecord, DbType, DbValue, Model, Query, QueryExecutor};
use crate::foundation::{Error, Result};
use crate::logging::RequestId;

mod cli;

pub(crate) use cli::audit_cli_registrar;

#[derive(Debug, Serialize, Deserialize, crate::Model)]
#[foundry(table = "audit_logs", audit = false)]
pub struct AuditLog {
    pub id: crate::ModelId<AuditLog>,
    pub event_type: String,
    pub subject_model: String,
    pub subject_table: String,
    pub subject_id: String,
    pub area: Option<String>,
    pub actor_guard: Option<String>,
    pub actor_id: Option<String>,
    pub request_id: Option<String>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub before_data: Option<serde_json::Value>,
    pub after_data: Option<serde_json::Value>,
    pub changes: Option<serde_json::Value>,
    pub created_at: crate::DateTime,
}

/// Attribution applied to model lifecycle audits within an async scope.
#[derive(Clone, Debug)]
pub struct AuditContext {
    area: String,
    actor: Option<Actor>,
    request_id: Option<RequestId>,
    ip: Option<IpAddr>,
    user_agent: Option<String>,
}

impl AuditContext {
    pub fn new(area: impl Into<String>) -> Self {
        Self::try_new(area).expect("audit area must be non-empty")
    }

    pub fn try_new(area: impl Into<String>) -> Result<Self> {
        let area = area.into();
        validate_audit_label("area", &area)?;
        Ok(Self {
            area,
            actor: None,
            request_id: None,
            ip: None,
            user_agent: None,
        })
    }

    pub fn with_actor(mut self, actor: Actor) -> Self {
        self.actor = Some(actor);
        self
    }

    pub fn with_request_id(mut self, request_id: RequestId) -> Self {
        self.request_id = Some(request_id);
        self
    }

    pub fn with_ip(mut self, ip: IpAddr) -> Self {
        self.ip = Some(ip);
        self
    }

    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    pub fn area(&self) -> &str {
        &self.area
    }

    pub fn actor(&self) -> Option<&Actor> {
        self.actor.as_ref()
    }
}

tokio::task_local! {
    static CURRENT_AUDIT_CONTEXT: AuditContext;
}

/// Run domain work with audit attribution independent of an HTTP route.
pub async fn scope_audit<F>(context: AuditContext, future: F) -> F::Output
where
    F: Future,
{
    CURRENT_AUDIT_CONTEXT.scope(context, future).await
}

/// Explicit audit event for domain actions that are not model lifecycle writes.
#[derive(Clone, Debug)]
pub struct AuditEntry {
    event_type: String,
    subject_model: String,
    subject_table: String,
    subject_id: String,
    area: Option<String>,
    before_data: Option<serde_json::Value>,
    after_data: Option<serde_json::Value>,
    changes: Option<serde_json::Value>,
}

impl AuditEntry {
    pub fn new(
        event_type: impl Into<String>,
        subject_table: impl Into<String>,
        subject_id: impl Into<String>,
    ) -> Self {
        let subject_table = subject_table.into();
        Self {
            event_type: event_type.into(),
            subject_model: subject_table.clone(),
            subject_table,
            subject_id: subject_id.into(),
            area: None,
            before_data: None,
            after_data: None,
            changes: None,
        }
    }

    pub fn subject_model(mut self, subject_model: impl Into<String>) -> Self {
        self.subject_model = subject_model.into();
        self
    }

    pub fn area(mut self, area: impl Into<String>) -> Self {
        self.area = Some(area.into());
        self
    }

    pub fn before(mut self, value: impl Serialize) -> Result<Self> {
        self.before_data = Some(serde_json::to_value(value).map_err(Error::other)?);
        Ok(self)
    }

    pub fn after(mut self, value: impl Serialize) -> Result<Self> {
        self.after_data = Some(serde_json::to_value(value).map_err(Error::other)?);
        Ok(self)
    }

    pub fn changes(mut self, value: impl Serialize) -> Result<Self> {
        self.changes = Some(serde_json::to_value(value).map_err(Error::other)?);
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuditEventType {
    Created,
    Updated,
    SoftDeleted,
    Restored,
    Deleted,
}

impl AuditEventType {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::SoftDeleted => "soft_deleted",
            Self::Restored => "restored",
            Self::Deleted => "deleted",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct AuditPayload {
    before_data: Option<serde_json::Value>,
    after_data: Option<serde_json::Value>,
    changes: Option<serde_json::Value>,
}

struct AuditRedactionPolicy {
    excluded: BTreeSet<String>,
    sensitive: BTreeSet<String>,
    redact_sensitive: bool,
}

const REDACTED_AUDIT_VALUE: &str = "[redacted]";
const SENSITIVE_FIELD_SEGMENTS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "credential",
    "authorization",
];

impl AuditRedactionPolicy {
    fn new(excluded_fields: &[&str], config: &crate::config::AuditConfig) -> Self {
        Self {
            excluded: excluded_fields
                .iter()
                .map(|field| normalize_audit_field_name(field))
                .collect(),
            sensitive: config
                .sensitive_fields
                .iter()
                .map(|field| normalize_audit_field_name(field))
                .collect(),
            redact_sensitive: config.redact_sensitive_fields,
        }
    }

    fn excluded(&self, field: &str) -> bool {
        self.excluded.contains(&normalize_audit_field_name(field))
    }

    fn sensitive(&self, field: &str) -> bool {
        if !self.redact_sensitive {
            return false;
        }

        let normalized = normalize_audit_field_name(field);
        if self.sensitive.contains(&normalized) {
            return true;
        }

        let padded = format!("_{normalized}_");
        SENSITIVE_FIELD_SEGMENTS
            .iter()
            .any(|segment| padded.contains(&format!("_{segment}_")))
    }

    fn value(&self, field: &str, value: &DbValue) -> serde_json::Value {
        if self.sensitive(field) {
            serde_json::Value::String(REDACTED_AUDIT_VALUE.to_string())
        } else {
            self.redact_json_value(db_value_to_json(value))
        }
    }

    fn redact_json_value(&self, value: serde_json::Value) -> serde_json::Value {
        if !self.redact_sensitive {
            return value;
        }

        match value {
            serde_json::Value::Object(values) => serde_json::Value::Object(
                values
                    .into_iter()
                    .map(|(key, value)| {
                        let value = if self.sensitive(&key) {
                            serde_json::Value::String(REDACTED_AUDIT_VALUE.to_string())
                        } else {
                            self.redact_json_value(value)
                        };
                        (key, value)
                    })
                    .collect(),
            ),
            serde_json::Value::Array(values) => serde_json::Value::Array(
                values
                    .into_iter()
                    .map(|value| self.redact_json_value(value))
                    .collect(),
            ),
            value => value,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct AuditAttribution {
    area: Option<String>,
    actor: Option<Actor>,
    request_id: Option<String>,
    ip: Option<String>,
    user_agent: Option<String>,
}

/// Manual audit writer and retention API.
pub struct AuditManager {
    availability: AtomicU8,
    warned_missing: AtomicBool,
    config: crate::config::AuditConfig,
}

impl AuditManager {
    pub(crate) fn new(config: crate::config::AuditConfig) -> Self {
        Self {
            availability: AtomicU8::new(0),
            warned_missing: AtomicBool::new(false),
            config,
        }
    }

    pub(crate) fn active_for<M>(&self) -> bool
    where
        M: Model,
    {
        let request = crate::logging::current_request();
        self.active_for_request::<M>(request.as_ref())
    }

    pub(crate) fn active_for_request<M>(
        &self,
        request: Option<&crate::logging::CurrentRequest>,
    ) -> bool
    where
        M: Model,
    {
        M::audit_enabled()
            && M::table_meta().name() != "audit_logs"
            && current_audit_area(request).is_some()
    }

    async fn table_available<E>(&self, executor: &E) -> Result<bool>
    where
        E: QueryExecutor + ?Sized,
    {
        match self.availability.load(Ordering::Relaxed) {
            1 => return Ok(true),
            2 => return Ok(false),
            _ => {}
        }

        let rows = executor
            .raw_query(
                r#"
                SELECT
                    to_regclass('audit_logs')::TEXT AS audit_table,
                    EXISTS (
                        SELECT 1
                        FROM information_schema.columns
                        WHERE table_schema = current_schema()
                          AND table_name = 'audit_logs'
                          AND column_name = 'area'
                    ) AS audit_area_column
                "#,
                &[],
            )
            .await?;
        let table_exists = rows
            .first()
            .and_then(|row| row.get("audit_table"))
            .is_some_and(|value| !matches!(value, DbValue::Null(_)));
        let has_area_column = rows
            .first()
            .and_then(|row| row.get("audit_area_column"))
            .is_some_and(|value| matches!(value, DbValue::Bool(true)));
        let available = table_exists && has_area_column;

        self.availability
            .store(if available { 1 } else { 2 }, Ordering::Relaxed);

        if !available && !self.warned_missing.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                target: "foundry.audit",
                "audit_logs table or `area` column is missing; built-in audit logging is disabled until framework migrations are published and applied"
            );
        }

        Ok(available)
    }

    /// Write an explicit domain audit entry through any query executor.
    pub async fn record<E>(&self, executor: &E, entry: AuditEntry) -> Result<()>
    where
        E: QueryExecutor + ?Sized,
    {
        validate_audit_label("event type", &entry.event_type)?;
        validate_audit_label("subject model", &entry.subject_model)?;
        validate_audit_label("subject table", &entry.subject_table)?;
        validate_audit_label("subject ID", &entry.subject_id)?;
        if let Some(area) = entry.area.as_deref() {
            validate_audit_label("area", area)?;
        }
        if !self.table_available(executor).await? {
            return Err(Error::message(
                "audit_logs table or `area` column is unavailable; publish and run framework migrations before recording manual audit entries",
            ));
        }

        let redaction = AuditRedactionPolicy::new(&[], &self.config);
        let mut attribution = current_audit_attribution(None);
        if entry.area.is_some() {
            attribution.area = entry.area;
        }
        let payload = AuditPayload {
            before_data: entry
                .before_data
                .map(|value| redaction.redact_json_value(value)),
            after_data: entry
                .after_data
                .map(|value| redaction.redact_json_value(value)),
            changes: entry
                .changes
                .map(|value| redaction.redact_json_value(value)),
        };

        insert_audit_row(
            executor,
            &entry.event_type,
            &entry.subject_model,
            &entry.subject_table,
            &entry.subject_id,
            &attribution,
            payload,
        )
        .await
    }

    /// Delete every audit row older than `cutoff`.
    pub async fn prune_before<E>(&self, executor: &E, cutoff: crate::DateTime) -> Result<u64>
    where
        E: QueryExecutor + ?Sized,
    {
        if !self.table_available(executor).await? {
            return Err(Error::message(
                "audit_logs table or `area` column is unavailable; publish and run framework migrations before pruning audit rows",
            ));
        }
        executor
            .raw_execute(
                "DELETE FROM audit_logs WHERE created_at < $1",
                &[DbValue::TimestampTz(cutoff)],
            )
            .await
    }

    /// Apply the configured retention window. A zero-day window is disabled.
    pub async fn prune_retention<E>(&self, executor: &E, now: crate::DateTime) -> Result<u64>
    where
        E: QueryExecutor + ?Sized,
    {
        if self.config.retention_days == 0 {
            return Ok(0);
        }
        self.prune_before(
            executor,
            now.sub_days(i64::from(self.config.retention_days)),
        )
        .await
    }

    pub fn retention_days(&self) -> u32 {
        self.config.retention_days
    }
}

fn current_audit_area(request: Option<&crate::logging::CurrentRequest>) -> Option<String> {
    CURRENT_AUDIT_CONTEXT
        .try_with(|context| context.area.clone())
        .ok()
        .or_else(|| request.and_then(|request| request.audit_area.clone()))
}

fn current_audit_attribution(hook_actor: Option<&Actor>) -> AuditAttribution {
    let request = crate::logging::current_request();
    let scoped = CURRENT_AUDIT_CONTEXT.try_with(Clone::clone).ok();
    AuditAttribution {
        area: scoped
            .as_ref()
            .map(|context| context.area.clone())
            .or_else(|| {
                request
                    .as_ref()
                    .and_then(|request| request.audit_area.clone())
            }),
        actor: scoped
            .as_ref()
            .and_then(|context| context.actor.clone())
            .or_else(|| hook_actor.cloned())
            .or_else(crate::logging::current_actor),
        request_id: scoped
            .as_ref()
            .and_then(|context| context.request_id.as_ref().map(ToString::to_string))
            .or_else(|| {
                request
                    .as_ref()
                    .and_then(|request| request.request_id.clone())
            }),
        ip: scoped
            .as_ref()
            .and_then(|context| context.ip.map(|ip| ip.to_string()))
            .or_else(|| {
                request
                    .as_ref()
                    .and_then(|request| request.ip.map(|ip| ip.to_string()))
            }),
        user_agent: scoped
            .as_ref()
            .and_then(|context| context.user_agent.clone())
            .or_else(|| {
                request
                    .as_ref()
                    .and_then(|request| request.user_agent.clone())
            }),
    }
}

fn validate_audit_label(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(Error::message(format!("audit {name} cannot be empty")));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_audit_row<E>(
    executor: &E,
    event_type: &str,
    subject_model: &str,
    subject_table: &str,
    subject_id: &str,
    attribution: &AuditAttribution,
    payload: AuditPayload,
) -> Result<()>
where
    E: QueryExecutor + ?Sized,
{
    Query::insert_into("audit_logs")
        .values([
            ("event_type", DbValue::Text(event_type.to_string())),
            ("subject_model", DbValue::Text(subject_model.to_string())),
            ("subject_table", DbValue::Text(subject_table.to_string())),
            ("subject_id", DbValue::Text(subject_id.to_string())),
            ("area", nullable_text(attribution.area.clone())),
            (
                "actor_guard",
                nullable_text(
                    attribution
                        .actor
                        .as_ref()
                        .map(|actor| actor.guard.to_string()),
                ),
            ),
            (
                "actor_id",
                nullable_text(attribution.actor.as_ref().map(|actor| actor.id.clone())),
            ),
            ("request_id", nullable_text(attribution.request_id.clone())),
            ("ip", nullable_text(attribution.ip.clone())),
            ("user_agent", nullable_text(attribution.user_agent.clone())),
            ("before_data", nullable_json(payload.before_data)),
            ("after_data", nullable_json(payload.after_data)),
            ("changes", nullable_json(payload.changes)),
        ])
        .execute(executor)
        .await?;
    Ok(())
}

pub(crate) async fn write_model_audit<M>(
    context: &crate::database::ModelHookContext<'_>,
    event_type: AuditEventType,
    before: Option<&DbRecord>,
    after: Option<&DbRecord>,
) -> Result<()>
where
    M: Model,
{
    let audit = context.app().audit()?;
    let request = crate::logging::current_request();
    let attribution = current_audit_attribution(context.actor());
    if !audit.active_for_request::<M>(request.as_ref())
        || !audit.table_available(context.transaction()).await?
    {
        return Ok(());
    }

    let redaction = AuditRedactionPolicy::new(M::audit_excluded_fields(), &audit.config);
    let payload = build_payload(event_type, before, after, &redaction);
    let subject_source = after.or(before).ok_or_else(|| {
        Error::message(format!(
            "audit logging for `{}` requires a before or after record",
            M::table_meta().name()
        ))
    })?;
    let subject_id = subject_id_for_record::<M>(subject_source)?;
    insert_audit_row(
        context.transaction(),
        event_type.as_str(),
        std::any::type_name::<M>(),
        M::table_meta().name(),
        &subject_id,
        &attribution,
        payload,
    )
    .await
}

pub(crate) fn record_with_assignments(
    current: &DbRecord,
    assignments: &[(crate::ColumnRef, crate::Expr)],
) -> DbRecord {
    let mut record = current.clone();
    for (column, expr) in assignments {
        if let crate::Expr::Value(value) = expr {
            record.insert(column.name.clone(), value.clone());
        }
    }
    record
}

fn subject_id_for_record<M>(record: &DbRecord) -> Result<String>
where
    M: Model,
{
    let primary_key = M::table_meta()
        .primary_key_column_info()
        .ok_or_else(|| Error::message("audit subject is missing a primary key column"))?;
    let value = record.get(primary_key.name).ok_or_else(|| {
        Error::message(format!(
            "audit subject record is missing primary key `{}`",
            primary_key.name
        ))
    })?;
    db_value_to_string(value)
}

fn nullable_text(value: Option<String>) -> DbValue {
    match value {
        Some(value) => DbValue::Text(value),
        None => DbValue::Null(DbType::Text),
    }
}

fn nullable_json(value: Option<serde_json::Value>) -> DbValue {
    match value {
        Some(value) => DbValue::Json(value),
        None => DbValue::Null(DbType::Json),
    }
}

fn build_payload(
    event_type: AuditEventType,
    before: Option<&DbRecord>,
    after: Option<&DbRecord>,
    redaction: &AuditRedactionPolicy,
) -> AuditPayload {
    match event_type {
        AuditEventType::Created => AuditPayload {
            before_data: None,
            after_data: after.map(|record| record_to_json(record, redaction)),
            changes: None,
        },
        AuditEventType::Deleted => AuditPayload {
            before_data: before.map(|record| record_to_json(record, redaction)),
            after_data: None,
            changes: None,
        },
        AuditEventType::Updated | AuditEventType::SoftDeleted | AuditEventType::Restored => {
            let before_data = before.map(|record| record_to_json(record, redaction));
            let after_data = after.map(|record| record_to_json(record, redaction));
            AuditPayload {
                changes: build_changes(before, after, redaction),
                before_data,
                after_data,
            }
        }
    }
}

fn build_changes(
    before: Option<&DbRecord>,
    after: Option<&DbRecord>,
    redaction: &AuditRedactionPolicy,
) -> Option<serde_json::Value> {
    let (Some(before), Some(after)) = (before, after) else {
        return None;
    };

    let mut keys = BTreeSet::new();
    for (key, _) in before.iter() {
        if !redaction.excluded(key) {
            keys.insert(key.clone());
        }
    }
    for (key, _) in after.iter() {
        if !redaction.excluded(key) {
            keys.insert(key.clone());
        }
    }

    let mut changes = serde_json::Map::new();
    for key in keys {
        let raw_before = before.get(&key).map(db_value_to_json).unwrap_or_default();
        let raw_after = after.get(&key).map(db_value_to_json).unwrap_or_default();
        if raw_before != raw_after {
            let before_value = before
                .get(&key)
                .map(|value| redaction.value(&key, value))
                .unwrap_or_default();
            let after_value = after
                .get(&key)
                .map(|value| redaction.value(&key, value))
                .unwrap_or_default();
            changes.insert(
                key,
                serde_json::json!({
                    "before": before_value,
                    "after": after_value,
                }),
            );
        }
    }

    if changes.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(changes))
    }
}

fn record_to_json(record: &DbRecord, redaction: &AuditRedactionPolicy) -> serde_json::Value {
    let mut values = serde_json::Map::new();
    for (key, value) in record.iter() {
        if redaction.excluded(key) {
            continue;
        }
        values.insert(key.clone(), redaction.value(key, value));
    }
    serde_json::Value::Object(values)
}

fn normalize_audit_field_name(field: &str) -> String {
    let mut normalized = String::new();
    let mut previous_was_separator = true;
    let mut previous_was_lower_or_digit = false;

    for ch in field.chars() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() && previous_was_lower_or_digit && !previous_was_separator {
                normalized.push('_');
            }
            normalized.push(ch.to_ascii_lowercase());
            previous_was_separator = false;
            previous_was_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        } else if !previous_was_separator {
            normalized.push('_');
            previous_was_separator = true;
            previous_was_lower_or_digit = false;
        }
    }

    normalized.trim_matches('_').to_string()
}

fn db_value_to_json(value: &DbValue) -> serde_json::Value {
    match value {
        DbValue::Null(_) => serde_json::Value::Null,
        DbValue::Int16(value) => serde_json::json!(value),
        DbValue::Int32(value) => serde_json::json!(value),
        DbValue::Int64(value) => serde_json::json!(value),
        DbValue::Bool(value) => serde_json::json!(value),
        DbValue::Float32(value) => serde_json::json!(value),
        DbValue::Float64(value) => serde_json::json!(value),
        DbValue::Numeric(value) => serde_json::Value::String(value.to_string()),
        DbValue::Text(value) => serde_json::Value::String(value.clone()),
        DbValue::Json(value) => value.clone(),
        DbValue::Uuid(value) => serde_json::Value::String(value.to_string()),
        DbValue::TimestampTz(value) => serde_json::Value::String(value.to_string()),
        DbValue::Timestamp(value) => serde_json::Value::String(value.to_string()),
        DbValue::Date(value) => serde_json::Value::String(value.to_string()),
        DbValue::Time(value) => serde_json::Value::String(value.to_string()),
        DbValue::Bytea(value) => {
            serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(value))
        }
        DbValue::Int16Array(value) => serde_json::json!(value),
        DbValue::Int32Array(value) => serde_json::json!(value),
        DbValue::Int64Array(value) => serde_json::json!(value),
        DbValue::BoolArray(value) => serde_json::json!(value),
        DbValue::Float32Array(value) => serde_json::json!(value),
        DbValue::Float64Array(value) => serde_json::json!(value),
        DbValue::NumericArray(value) => serde_json::Value::Array(
            value
                .iter()
                .map(|entry| serde_json::Value::String(entry.to_string()))
                .collect(),
        ),
        DbValue::TextArray(value) => serde_json::json!(value),
        DbValue::JsonArray(value) => serde_json::Value::Array(value.clone()),
        DbValue::UuidArray(value) => serde_json::Value::Array(
            value
                .iter()
                .map(|entry| serde_json::Value::String(entry.to_string()))
                .collect(),
        ),
        DbValue::TimestampTzArray(value) => serde_json::Value::Array(
            value
                .iter()
                .map(|entry| serde_json::Value::String(entry.to_string()))
                .collect(),
        ),
        DbValue::TimestampArray(value) => serde_json::Value::Array(
            value
                .iter()
                .map(|entry| serde_json::Value::String(entry.to_string()))
                .collect(),
        ),
        DbValue::DateArray(value) => serde_json::Value::Array(
            value
                .iter()
                .map(|entry| serde_json::Value::String(entry.to_string()))
                .collect(),
        ),
        DbValue::TimeArray(value) => serde_json::Value::Array(
            value
                .iter()
                .map(|entry| serde_json::Value::String(entry.to_string()))
                .collect(),
        ),
        DbValue::ByteaArray(value) => serde_json::Value::Array(
            value
                .iter()
                .map(|entry| {
                    serde_json::Value::String(
                        base64::engine::general_purpose::STANDARD.encode(entry),
                    )
                })
                .collect(),
        ),
    }
}

fn db_value_to_string(value: &DbValue) -> Result<String> {
    Ok(match value {
        DbValue::Null(_) => {
            return Err(Error::message(
                "audit subject primary key cannot be null after persistence",
            ));
        }
        DbValue::Int16(value) => value.to_string(),
        DbValue::Int32(value) => value.to_string(),
        DbValue::Int64(value) => value.to_string(),
        DbValue::Bool(value) => value.to_string(),
        DbValue::Float32(value) => value.to_string(),
        DbValue::Float64(value) => value.to_string(),
        DbValue::Numeric(value) => value.to_string(),
        DbValue::Text(value) => value.clone(),
        DbValue::Json(value) => value.to_string(),
        DbValue::Uuid(value) => value.to_string(),
        DbValue::TimestampTz(value) => value.to_string(),
        DbValue::Timestamp(value) => value.to_string(),
        DbValue::Date(value) => value.to_string(),
        DbValue::Time(value) => value.to_string(),
        DbValue::Bytea(value) => base64::engine::general_purpose::STANDARD.encode(value),
        DbValue::Int16Array(_)
        | DbValue::Int32Array(_)
        | DbValue::Int64Array(_)
        | DbValue::BoolArray(_)
        | DbValue::Float32Array(_)
        | DbValue::Float64Array(_)
        | DbValue::NumericArray(_)
        | DbValue::TextArray(_)
        | DbValue::JsonArray(_)
        | DbValue::UuidArray(_)
        | DbValue::TimestampTzArray(_)
        | DbValue::TimestampArray(_)
        | DbValue::DateArray(_)
        | DbValue::TimeArray(_)
        | DbValue::ByteaArray(_) => {
            return Err(Error::message(
                "audit subject primary key cannot use an array value",
            ));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{
        build_payload, record_with_assignments, AuditEventType, AuditRedactionPolicy,
        REDACTED_AUDIT_VALUE,
    };
    use crate::config::AuditConfig;
    use crate::{ColumnRef, DbRecord, DbType, DbValue, Expr};

    fn record(entries: &[(&str, DbValue)]) -> DbRecord {
        let mut record = DbRecord::new();
        for (key, value) in entries {
            record.insert(*key, value.clone());
        }
        record
    }

    fn policy(excluded_fields: &[&str]) -> AuditRedactionPolicy {
        AuditRedactionPolicy::new(excluded_fields, &AuditConfig::default())
    }

    fn unredacted_policy(excluded_fields: &[&str]) -> AuditRedactionPolicy {
        let config = AuditConfig {
            redact_sensitive_fields: false,
            ..AuditConfig::default()
        };
        AuditRedactionPolicy::new(excluded_fields, &config)
    }

    #[test]
    fn created_payload_uses_after_data_only() {
        let after = record(&[
            ("id", DbValue::Int64(1)),
            ("title", DbValue::Text("Hello".into())),
        ]);

        let payload = build_payload(AuditEventType::Created, None, Some(&after), &policy(&[]));

        assert!(payload.before_data.is_none());
        assert_eq!(payload.after_data.unwrap()["title"], "Hello");
        assert!(payload.changes.is_none());
    }

    #[test]
    fn updated_payload_tracks_dirty_fields_only() {
        let before = record(&[
            ("id", DbValue::Int64(1)),
            ("title", DbValue::Text("Before".into())),
            ("updated_at", DbValue::Text("old".into())),
        ]);
        let after = record(&[
            ("id", DbValue::Int64(1)),
            ("title", DbValue::Text("After".into())),
            ("updated_at", DbValue::Text("new".into())),
        ]);

        let payload = build_payload(
            AuditEventType::Updated,
            Some(&before),
            Some(&after),
            &policy(&["updated_at"]),
        );

        let changes = payload.changes.unwrap();
        assert_eq!(changes["title"]["before"], "Before");
        assert_eq!(changes["title"]["after"], "After");
        assert!(changes.get("updated_at").is_none());
    }

    #[test]
    fn deleted_payload_uses_before_data_only() {
        let before = record(&[
            ("id", DbValue::Int64(1)),
            ("title", DbValue::Text("Gone".into())),
        ]);

        let payload = build_payload(AuditEventType::Deleted, Some(&before), None, &policy(&[]));

        assert_eq!(payload.before_data.unwrap()["title"], "Gone");
        assert!(payload.after_data.is_none());
        assert!(payload.changes.is_none());
    }

    #[test]
    fn soft_delete_payload_marks_deleted_at_change() {
        let before = record(&[
            ("id", DbValue::Int64(1)),
            ("deleted_at", DbValue::Null(DbType::TimestampTz)),
        ]);
        let after = record_with_assignments(
            &before,
            &[(
                ColumnRef::new("posts", "deleted_at").typed(DbType::TimestampTz),
                Expr::value(DbValue::Text("2026-04-22T12:00:00Z".into())),
            )],
        );

        let payload = build_payload(
            AuditEventType::SoftDeleted,
            Some(&before),
            Some(&after),
            &policy(&[]),
        );

        let changes = payload.changes.unwrap();
        assert!(changes.get("deleted_at").is_some());
    }

    #[test]
    fn restored_payload_marks_deleted_at_change() {
        let before = record(&[
            ("id", DbValue::Int64(1)),
            ("deleted_at", DbValue::Text("2026-04-22T12:00:00Z".into())),
        ]);
        let after = record(&[
            ("id", DbValue::Int64(1)),
            ("deleted_at", DbValue::Null(DbType::TimestampTz)),
        ]);

        let payload = build_payload(
            AuditEventType::Restored,
            Some(&before),
            Some(&after),
            &policy(&[]),
        );

        let changes = payload.changes.unwrap();
        assert!(changes.get("deleted_at").is_some());
    }

    #[test]
    fn default_audit_redaction_masks_sensitive_fields_without_removing_keys() {
        let after = record(&[
            ("id", DbValue::Int64(1)),
            ("password_hash", DbValue::Text("hash-secret".into())),
            ("refreshToken", DbValue::Text("token-secret".into())),
            ("title", DbValue::Text("Visible".into())),
        ]);

        let payload = build_payload(AuditEventType::Created, None, Some(&after), &policy(&[]));
        let after_data = payload.after_data.unwrap();

        assert_eq!(after_data["title"], "Visible");
        assert_eq!(after_data["password_hash"], REDACTED_AUDIT_VALUE);
        assert_eq!(after_data["refreshToken"], REDACTED_AUDIT_VALUE);
        assert!(!after_data.to_string().contains("hash-secret"));
        assert!(!after_data.to_string().contains("token-secret"));
    }

    #[test]
    fn audit_redaction_recurses_through_json_objects_and_arrays() {
        let after = record(&[(
            "settings",
            DbValue::Json(serde_json::json!({
                "theme": "dark",
                "api_key": "top-level-secret",
                "integrations": [
                    {
                        "name": "billing",
                        "credentials": {
                            "accessToken": "nested-token",
                            "endpoint": "https://example.test"
                        }
                    }
                ]
            })),
        )]);

        let payload = build_payload(AuditEventType::Created, None, Some(&after), &policy(&[]));
        let settings = &payload.after_data.unwrap()["settings"];

        assert_eq!(settings["theme"], "dark");
        assert_eq!(settings["api_key"], REDACTED_AUDIT_VALUE);
        assert_eq!(
            settings["integrations"][0]["credentials"],
            REDACTED_AUDIT_VALUE
        );
        assert!(!settings.to_string().contains("top-level-secret"));
        assert!(!settings.to_string().contains("nested-token"));
    }

    #[test]
    fn sensitive_audit_changes_record_redacted_markers_when_raw_values_change() {
        let before = record(&[
            ("id", DbValue::Int64(1)),
            ("api_key", DbValue::Text("old-key".into())),
        ]);
        let after = record(&[
            ("id", DbValue::Int64(1)),
            ("api_key", DbValue::Text("new-key".into())),
        ]);

        let payload = build_payload(
            AuditEventType::Updated,
            Some(&before),
            Some(&after),
            &policy(&[]),
        );
        let changes = payload.changes.unwrap();

        assert_eq!(changes["api_key"]["before"], REDACTED_AUDIT_VALUE);
        assert_eq!(changes["api_key"]["after"], REDACTED_AUDIT_VALUE);
        assert!(!changes.to_string().contains("old-key"));
        assert!(!changes.to_string().contains("new-key"));
    }

    #[test]
    fn explicit_audit_exclude_still_removes_fields_entirely() {
        let after = record(&[
            ("id", DbValue::Int64(1)),
            ("secret", DbValue::Text("hidden".into())),
        ]);

        let payload = build_payload(
            AuditEventType::Created,
            None,
            Some(&after),
            &policy(&["secret"]),
        );

        assert!(payload.after_data.unwrap().get("secret").is_none());
    }

    #[test]
    fn audit_redaction_can_be_disabled_by_config() {
        let after = record(&[
            ("id", DbValue::Int64(1)),
            ("password", DbValue::Text("visible-for-test".into())),
            (
                "settings",
                DbValue::Json(serde_json::json!({"api_key": "nested-visible"})),
            ),
        ]);

        let payload = build_payload(
            AuditEventType::Created,
            None,
            Some(&after),
            &unredacted_policy(&[]),
        );

        let after_data = payload.after_data.unwrap();
        assert_eq!(after_data["password"], "visible-for-test");
        assert_eq!(after_data["settings"]["api_key"], "nested-visible");
    }
}
