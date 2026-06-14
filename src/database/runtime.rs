use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock, RwLock};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{
    DateTime as ChronoDateTime, NaiveDate as ChronoDate, NaiveDateTime as ChronoNaiveDateTime,
    NaiveTime as ChronoTime, Utc as ChronoUtc,
};
use futures_util::stream::{self, BoxStream};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgConnection, PgPoolOptions, PgRow};
use sqlx::types::BigDecimal;
use sqlx::{Column as _, PgPool, Postgres, Row, Transaction, TypeInfo as _};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use uuid::Uuid;

use crate::config::{DatabaseConfig, ObservabilityConfig};
use crate::foundation::{Error, Result};
use crate::logging::{catch_future_panic, panic_payload_message};
use crate::support::sync::{lock_unpoisoned, read_unpoisoned, write_unpoisoned};
use crate::support::{Date, DateTime, LocalDateTime, Time};

use super::ast::{DbType, DbValue, Numeric};
use super::compiler::CompiledSql;

// ---------------------------------------------------------------------------
// SQL query logging
// ---------------------------------------------------------------------------

const SLOW_QUERY_LOG_CAPACITY: usize = 100;

#[derive(Clone, Debug)]
pub(crate) struct SqlLogConfig {
    pub capture_enabled: bool,
    pub log_queries: bool,
    pub log_query_bindings: bool,
    pub redact_sql_literals: bool,
    pub slow_threshold: Option<Duration>,
    pub slow_query_retention: usize,
}

impl SqlLogConfig {
    pub fn from_configs(
        config: &DatabaseConfig,
        observability: Option<&ObservabilityConfig>,
    ) -> Self {
        let capture_enabled = observability
            .map(|config| config.capture_enabled)
            .unwrap_or(true);
        Self {
            capture_enabled,
            log_queries: config.log_queries,
            log_query_bindings: config.log_query_bindings,
            redact_sql_literals: config.redact_sql_literals,
            slow_threshold: if capture_enabled && config.slow_query_threshold_ms > 0 {
                Some(Duration::from_millis(config.slow_query_threshold_ms))
            } else {
                None
            },
            slow_query_retention: config.slow_query_retention,
        }
    }

    #[allow(dead_code)]
    pub fn disabled() -> Self {
        Self {
            capture_enabled: false,
            log_queries: false,
            log_query_bindings: false,
            redact_sql_literals: true,
            slow_threshold: None,
            slow_query_retention: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SlowQueryEntry {
    pub sql: String,
    pub duration_ms: u64,
    pub label: Option<String>,
    pub request_id: Option<String>,
    pub trace_id: Option<String>,
    pub recorded_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SqlObservabilitySnapshot {
    pub stats: SqlObservabilityStats,
    pub top_slowest: Vec<SlowQueryEntry>,
    pub n_plus_one_suspects: Vec<NPlusOneSuspect>,
    pub slow_queries: Vec<SlowQueryEntry>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SqlObservabilityStats {
    pub retained_count: usize,
    pub capacity: usize,
    pub slow_query_threshold_ms: u64,
    pub max_duration_ms: Option<u64>,
    pub avg_duration_ms: Option<u64>,
    pub latest_recorded_at: Option<String>,
    pub n_plus_one_suspect_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NPlusOneSuspect {
    pub method: String,
    pub path: String,
    pub request_id: Option<String>,
    pub trace_id: Option<String>,
    pub fingerprint: String,
    pub repeat_count: u64,
    pub total_duration_ms: u64,
    pub max_duration_ms: u64,
    pub avg_duration_ms: u64,
    pub rows_total: u64,
    pub labels: Vec<String>,
    pub kinds: Vec<String>,
    pub sample_sql: String,
    pub first_recorded_at: String,
    pub latest_recorded_at: String,
}

static SLOW_QUERY_LOG: OnceLock<std::sync::Mutex<VecDeque<SlowQueryEntry>>> = OnceLock::new();
static N_PLUS_ONE_LOG: OnceLock<std::sync::Mutex<VecDeque<NPlusOneSuspect>>> = OnceLock::new();

tokio::task_local! {
    static SQL_QUERY_TRACE: RefCell<SqlQueryTrace>;
}

fn slow_query_log() -> &'static std::sync::Mutex<VecDeque<SlowQueryEntry>> {
    SLOW_QUERY_LOG
        .get_or_init(|| std::sync::Mutex::new(VecDeque::with_capacity(SLOW_QUERY_LOG_CAPACITY)))
}

fn n_plus_one_log() -> &'static std::sync::Mutex<VecDeque<NPlusOneSuspect>> {
    N_PLUS_ONE_LOG.get_or_init(|| std::sync::Mutex::new(VecDeque::with_capacity(100)))
}

fn record_slow_query(
    sql: &str,
    duration_ms: u64,
    label: Option<&str>,
    retention: usize,
    redact_literals: bool,
) {
    if retention == 0 {
        return;
    }

    let mut log = lock_unpoisoned(slow_query_log(), "slow query log");
    while log.len() >= retention {
        log.pop_front();
    }
    let trace = crate::logging::current_trace_context();
    log.push_back(SlowQueryEntry {
        sql: sql_for_observability(sql, redact_literals),
        duration_ms,
        label: label.map(|s| s.to_string()),
        request_id: trace
            .as_ref()
            .and_then(|context| context.request_id.clone()),
        trace_id: trace.map(|context| context.trace_id),
        recorded_at: ChronoUtc::now().to_rfc3339(),
    });
}

#[cfg(test)]
pub fn recent_slow_queries() -> Vec<SlowQueryEntry> {
    lock_unpoisoned(slow_query_log(), "slow query log")
        .iter()
        .cloned()
        .collect()
}

fn recent_slow_queries_with_retention(retention: usize) -> Vec<SlowQueryEntry> {
    if retention == 0 {
        return Vec::new();
    }

    lock_unpoisoned(slow_query_log(), "slow query log")
        .iter()
        .rev()
        .take(retention)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn recent_n_plus_one_suspects() -> Vec<NPlusOneSuspect> {
    lock_unpoisoned(n_plus_one_log(), "n+1 query log")
        .iter()
        .cloned()
        .collect()
}

pub(crate) fn sql_observability_snapshot(
    slow_query_threshold_ms: u64,
    slow_query_retention: usize,
) -> SqlObservabilitySnapshot {
    let slow_queries = recent_slow_queries_with_retention(slow_query_retention);
    let n_plus_one_suspects = recent_n_plus_one_suspects();
    let retained_count = slow_queries.len();
    let max_duration_ms = slow_queries.iter().map(|query| query.duration_ms).max();
    let avg_duration_ms = if slow_queries.is_empty() {
        None
    } else {
        Some(
            slow_queries
                .iter()
                .map(|query| query.duration_ms)
                .sum::<u64>()
                / slow_queries.len() as u64,
        )
    };
    let latest_recorded_at = slow_queries
        .iter()
        .map(|query| query.recorded_at.clone())
        .max();
    let mut top_slowest = slow_queries.clone();
    top_slowest.sort_by(|left, right| {
        right
            .duration_ms
            .cmp(&left.duration_ms)
            .then_with(|| right.recorded_at.cmp(&left.recorded_at))
    });

    SqlObservabilitySnapshot {
        stats: SqlObservabilityStats {
            retained_count,
            capacity: slow_query_retention,
            slow_query_threshold_ms,
            max_duration_ms,
            avg_duration_ms,
            latest_recorded_at,
            n_plus_one_suspect_count: n_plus_one_suspects.len(),
        },
        top_slowest,
        n_plus_one_suspects,
        slow_queries,
    }
}

#[derive(Clone, Debug)]
struct SqlQueryTraceConfig {
    enabled: bool,
    min_repeats: u64,
    retention: usize,
    redact_sql_literals: bool,
    method: String,
    path: String,
    request_id: Option<String>,
    trace_id: Option<String>,
}

impl SqlQueryTraceConfig {
    fn from_configs(
        database: Option<DatabaseConfig>,
        observability: Option<ObservabilityConfig>,
        method: String,
        path: String,
        request_id: Option<String>,
    ) -> Self {
        let capture_enabled = observability
            .as_ref()
            .map(|config| config.capture_enabled)
            .unwrap_or(true);
        let Some(config) = database else {
            return Self {
                enabled: false,
                min_repeats: 10,
                retention: 0,
                redact_sql_literals: true,
                method,
                path,
                request_id,
                trace_id: None,
            };
        };

        let trace = crate::logging::current_trace_context();
        Self {
            enabled: capture_enabled && config.n_plus_one_detection,
            min_repeats: config.n_plus_one_min_repeats.max(1),
            retention: config.n_plus_one_retention,
            redact_sql_literals: config.redact_sql_literals,
            method,
            path,
            request_id: request_id.or_else(|| {
                trace
                    .as_ref()
                    .and_then(|context| context.request_id.clone())
            }),
            trace_id: trace.map(|context| context.trace_id),
        }
    }
}

#[derive(Debug)]
struct SqlQueryTrace {
    config: SqlQueryTraceConfig,
    groups: HashMap<String, SqlQueryGroup>,
    finished: bool,
}

impl SqlQueryTrace {
    fn new(config: SqlQueryTraceConfig) -> Self {
        Self {
            config,
            groups: HashMap::new(),
            finished: false,
        }
    }

    fn record(&mut self, sql: &str, duration_ms: u64, label: Option<&str>, kind: &str, rows: u64) {
        if !self.config.enabled || self.config.retention == 0 {
            return;
        }

        let fingerprint = sql_fingerprint(sql);
        let recorded_at = ChronoUtc::now().to_rfc3339();
        self.groups
            .entry(fingerprint.clone())
            .and_modify(|group| group.record(duration_ms, label, kind, rows, &recorded_at))
            .or_insert_with(|| {
                SqlQueryGroup::new(
                    fingerprint,
                    &sql_for_observability(sql, self.config.redact_sql_literals),
                    duration_ms,
                    label,
                    kind,
                    rows,
                    recorded_at,
                )
            });
    }

    fn finish(&mut self) -> Vec<NPlusOneSuspect> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;

        let mut suspects = std::mem::take(&mut self.groups)
            .into_values()
            .filter(|group| group.repeat_count >= self.config.min_repeats)
            .map(|group| group.into_suspect(&self.config))
            .collect::<Vec<_>>();
        suspects.sort_by(|left, right| {
            right
                .repeat_count
                .cmp(&left.repeat_count)
                .then_with(|| right.total_duration_ms.cmp(&left.total_duration_ms))
                .then_with(|| left.path.cmp(&right.path))
                .then_with(|| left.fingerprint.cmp(&right.fingerprint))
        });
        suspects
    }
}

impl Drop for SqlQueryTrace {
    fn drop(&mut self) {
        if self.finished {
            return;
        }

        let retention = self.config.retention;
        let suspects = self.finish();
        record_n_plus_one_suspects(suspects, retention);
    }
}

#[derive(Debug)]
struct SqlQueryGroup {
    fingerprint: String,
    sample_sql: String,
    repeat_count: u64,
    total_duration_ms: u64,
    max_duration_ms: u64,
    rows_total: u64,
    labels: BTreeSet<String>,
    kinds: BTreeSet<String>,
    first_recorded_at: String,
    latest_recorded_at: String,
}

impl SqlQueryGroup {
    fn new(
        fingerprint: String,
        sql: &str,
        duration_ms: u64,
        label: Option<&str>,
        kind: &str,
        rows: u64,
        recorded_at: String,
    ) -> Self {
        let mut group = Self {
            fingerprint,
            sample_sql: sql.to_string(),
            repeat_count: 0,
            total_duration_ms: 0,
            max_duration_ms: 0,
            rows_total: 0,
            labels: BTreeSet::new(),
            kinds: BTreeSet::new(),
            first_recorded_at: recorded_at.clone(),
            latest_recorded_at: recorded_at,
        };
        let recorded_at = group.latest_recorded_at.clone();
        group.record(duration_ms, label, kind, rows, &recorded_at);
        group
    }

    fn record(
        &mut self,
        duration_ms: u64,
        label: Option<&str>,
        kind: &str,
        rows: u64,
        recorded_at: &str,
    ) {
        self.repeat_count += 1;
        self.total_duration_ms += duration_ms;
        self.max_duration_ms = self.max_duration_ms.max(duration_ms);
        self.rows_total += rows;
        if let Some(label) = label.filter(|value| !value.trim().is_empty()) {
            self.labels.insert(label.to_string());
        }
        self.kinds.insert(kind.to_string());
        self.latest_recorded_at = recorded_at.to_string();
    }

    fn into_suspect(self, config: &SqlQueryTraceConfig) -> NPlusOneSuspect {
        NPlusOneSuspect {
            method: config.method.clone(),
            path: config.path.clone(),
            request_id: config.request_id.clone(),
            trace_id: config.trace_id.clone(),
            fingerprint: self.fingerprint,
            repeat_count: self.repeat_count,
            total_duration_ms: self.total_duration_ms,
            max_duration_ms: self.max_duration_ms,
            avg_duration_ms: self.total_duration_ms / self.repeat_count,
            rows_total: self.rows_total,
            labels: self.labels.into_iter().collect(),
            kinds: self.kinds.into_iter().collect(),
            sample_sql: self.sample_sql,
            first_recorded_at: self.first_recorded_at,
            latest_recorded_at: self.latest_recorded_at,
        }
    }
}

pub(crate) async fn scope_http_sql_query_trace<F, T>(
    database: Option<DatabaseConfig>,
    observability: Option<ObservabilityConfig>,
    method: String,
    path: String,
    request_id: Option<String>,
    future: F,
) -> T
where
    F: Future<Output = T>,
{
    let trace_config =
        SqlQueryTraceConfig::from_configs(database, observability, method, path, request_id);
    if !trace_config.enabled || trace_config.retention == 0 {
        return future.await;
    }

    SQL_QUERY_TRACE
        .scope(RefCell::new(SqlQueryTrace::new(trace_config)), async move {
            let output = future.await;
            finish_current_sql_query_trace();
            output
        })
        .await
}

fn finish_current_sql_query_trace() {
    let _ = SQL_QUERY_TRACE.try_with(|trace| {
        let mut trace = trace.borrow_mut();
        let retention = trace.config.retention;
        let suspects = trace.finish();
        record_n_plus_one_suspects(suspects, retention);
    });
}

fn record_sql_observation(sql: &str, duration_ms: u64, label: Option<&str>, kind: &str, rows: u64) {
    let _ = SQL_QUERY_TRACE.try_with(|trace| {
        trace
            .borrow_mut()
            .record(sql, duration_ms, label, kind, rows);
    });
}

fn record_n_plus_one_suspects(suspects: Vec<NPlusOneSuspect>, retention: usize) {
    if suspects.is_empty() || retention == 0 {
        return;
    }

    let mut log = lock_unpoisoned(n_plus_one_log(), "n+1 query log");
    for suspect in suspects {
        log.push_back(suspect);
        while log.len() > retention {
            log.pop_front();
        }
    }
}

pub(crate) fn sql_fingerprint(sql: &str) -> String {
    let mut normalized = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    let mut pending_space = false;

    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            pending_space = true;
            continue;
        }

        if ch == '\'' {
            push_normalized_char(&mut normalized, &mut pending_space, '?');
            while let Some(next) = chars.next() {
                if next == '\'' {
                    if chars.peek() == Some(&'\'') {
                        chars.next();
                        continue;
                    }
                    break;
                }
            }
            continue;
        }

        if ch == '$' && matches!(chars.peek(), Some(next) if next.is_ascii_digit()) {
            push_normalized_char(&mut normalized, &mut pending_space, '?');
            while matches!(chars.peek(), Some(next) if next.is_ascii_digit()) {
                chars.next();
            }
            continue;
        }

        if ch == '?' {
            push_normalized_char(&mut normalized, &mut pending_space, '?');
            continue;
        }

        let previous_is_identifier = !pending_space
            && normalized
                .chars()
                .last()
                .is_some_and(|last| last.is_ascii_alphanumeric() || last == '_');
        if ch.is_ascii_digit() && !previous_is_identifier {
            push_normalized_char(&mut normalized, &mut pending_space, '?');
            while matches!(chars.peek(), Some(next) if next.is_ascii_digit() || *next == '.') {
                chars.next();
            }
            continue;
        }

        push_normalized_char(&mut normalized, &mut pending_space, ch.to_ascii_lowercase());
    }

    normalized.trim().to_string()
}

fn sql_for_observability(sql: &str, redact_literals: bool) -> String {
    if redact_literals {
        redact_sql_literals(sql)
    } else {
        sql.to_string()
    }
}

pub(crate) fn redact_sql_literals(sql: &str) -> String {
    let mut redacted = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    let mut previous = None;

    while let Some(ch) = chars.next() {
        if ch == '-' && chars.peek() == Some(&'-') {
            chars.next();
            redacted.push_str("-- redacted");
            for next in chars.by_ref() {
                if next == '\n' {
                    redacted.push('\n');
                    previous = Some('\n');
                    break;
                }
            }
            continue;
        }

        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            redacted.push_str("/* redacted */");
            let mut saw_star = false;
            for next in chars.by_ref() {
                if saw_star && next == '/' {
                    break;
                }
                saw_star = next == '*';
            }
            previous = Some('/');
            continue;
        }

        if let Some(delimiter) = read_dollar_quote_delimiter(ch, &mut chars, previous) {
            redacted.push('?');
            consume_dollar_quoted_literal(&delimiter, &mut chars);
            previous = Some('?');
            continue;
        }

        if starts_single_quoted_literal(ch, chars.peek().copied(), previous) {
            if ch == '\'' {
                consume_single_quoted_literal(&mut chars);
            } else {
                chars.next();
                consume_single_quoted_literal(&mut chars);
            }
            redacted.push('?');
            previous = Some('?');
            continue;
        }

        if ch.is_ascii_digit() && !previous.is_some_and(is_identifier_char) {
            redacted.push('?');
            while matches!(chars.peek(), Some(next) if next.is_ascii_digit() || *next == '.') {
                chars.next();
            }
            previous = Some('?');
            continue;
        }

        redacted.push(ch);
        previous = Some(ch);
    }

    redacted
}

fn starts_single_quoted_literal(ch: char, next: Option<char>, previous: Option<char>) -> bool {
    if ch == '\'' {
        return true;
    }
    matches!(ch, 'e' | 'E' | 'n' | 'N')
        && next == Some('\'')
        && !previous.is_some_and(is_identifier_char)
}

fn consume_single_quoted_literal(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(next) = chars.next() {
        if next == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                continue;
            }
            break;
        }
    }
}

fn read_dollar_quote_delimiter(
    first: char,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    previous: Option<char>,
) -> Option<String> {
    if first != '$' || previous.is_some_and(is_identifier_char) {
        return None;
    }

    let mut probe = chars.clone();
    let mut delimiter = String::from(first);
    while let Some(next) = probe.next() {
        delimiter.push(next);
        if next == '$' {
            *chars = probe;
            return Some(delimiter);
        }
        if !(next == '_' || next.is_ascii_alphanumeric()) {
            return None;
        }
    }
    None
}

fn consume_dollar_quoted_literal(
    delimiter: &str,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) {
    let mut pending = String::new();
    for next in chars.by_ref() {
        pending.push(next);
        if pending.ends_with(delimiter) {
            break;
        }
        while pending.len() > delimiter.len() {
            pending.remove(0);
        }
    }
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn push_normalized_char(normalized: &mut String, pending_space: &mut bool, ch: char) {
    if *pending_space && !normalized.is_empty() {
        normalized.push(' ');
    }
    normalized.push(ch);
    *pending_space = false;
}

pub type DbRecordStream<'a> = BoxStream<'a, Result<DbRecord>>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueryExecutionOptions {
    pub timeout: Option<Duration>,
    pub label: Option<String>,
    /// When true, forces this query to use the write (primary) pool instead
    /// of the read replica. Useful for reads that must see the most recent writes.
    pub use_write_pool: bool,
}

impl QueryExecutionOptions {
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_write_pool(mut self) -> Self {
        self.use_write_pool = true;
        self
    }
}

#[derive(Clone)]
pub struct DatabaseManager {
    state: Arc<DatabaseState>,
}

enum DatabaseState {
    Disabled,
    Ready(DatabaseRuntime),
}

struct DatabaseRuntime {
    pool: PgPool,
    read_pool: Option<PgPool>,
    adapters: Arc<RwLock<BTreeMap<String, DbType>>>,
    sql_log: SqlLogConfig,
}

impl DatabaseRuntime {
    /// Returns the pool to use for read operations. Falls back to the write
    /// pool when no read replica is configured or when `force_write` is true.
    fn pool_for_reads(&self, force_write: bool) -> &PgPool {
        if force_write {
            &self.pool
        } else {
            self.read_pool.as_ref().unwrap_or(&self.pool)
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DbRecord {
    values: BTreeMap<String, DbValue>,
}

impl DbRecord {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, key: impl Into<String>, value: DbValue) {
        self.values.insert(key.into(), value);
    }

    pub fn get(&self, key: &str) -> Option<&DbValue> {
        self.values.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &DbValue)> {
        self.values.iter()
    }

    /// Extract a text value, returning empty string if missing or wrong type.
    pub fn text(&self, field: &str) -> String {
        match self.get(field) {
            Some(DbValue::Text(s)) => s.clone(),
            _ => String::new(),
        }
    }

    /// Extract a required text value.
    pub fn try_text(&self, field: &str) -> Result<String> {
        match self.get(field) {
            Some(DbValue::Text(s)) => Ok(s.clone()),
            Some(value) => Err(Error::message(format!(
                "database field `{field}` expected text, got {:?}",
                value.db_type()
            ))),
            None => Err(Error::message(format!(
                "database field `{field}` is missing"
            ))),
        }
    }

    /// Extract a text or UUID value as string.
    pub fn text_or_uuid(&self, field: &str) -> String {
        match self.get(field) {
            Some(DbValue::Text(s)) => s.clone(),
            Some(DbValue::Uuid(u)) => u.to_string(),
            _ => String::new(),
        }
    }

    /// Extract a required text or UUID value as string.
    pub fn try_text_or_uuid(&self, field: &str) -> Result<String> {
        match self.get(field) {
            Some(DbValue::Text(s)) => Ok(s.clone()),
            Some(DbValue::Uuid(u)) => Ok(u.to_string()),
            Some(value) => Err(Error::message(format!(
                "database field `{field}` expected text or uuid, got {:?}",
                value.db_type()
            ))),
            None => Err(Error::message(format!(
                "database field `{field}` is missing"
            ))),
        }
    }

    /// Extract an optional text value.
    pub fn optional_text(&self, field: &str) -> Option<String> {
        match self.get(field) {
            Some(DbValue::Text(s)) => Some(s.clone()),
            _ => None,
        }
    }
}

pub struct DatabaseTransaction {
    inner: Mutex<Option<Transaction<'static, Postgres>>>,
    adapters: Arc<RwLock<BTreeMap<String, DbType>>>,
    sql_log: SqlLogConfig,
}

#[derive(Clone)]
pub(crate) struct DatabaseSession {
    pool: PgPool,
    inner: Arc<Mutex<Option<PoolConnection<Postgres>>>>,
    adapters: Arc<RwLock<BTreeMap<String, DbType>>>,
    sql_log: SqlLogConfig,
}

#[async_trait]
pub trait QueryExecutor: Send + Sync {
    async fn raw_query_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<Vec<DbRecord>>;

    async fn raw_execute_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<u64>;

    fn stream_records<'a>(
        &'a self,
        compiled: CompiledSql,
        options: QueryExecutionOptions,
    ) -> DbRecordStream<'a>
    where
        Self: Sized,
    {
        fallback_stream(self, compiled, options)
    }

    async fn raw_query(&self, sql: &str, bindings: &[DbValue]) -> Result<Vec<DbRecord>> {
        self.raw_query_with(sql, bindings, QueryExecutionOptions::default())
            .await
    }

    async fn raw_execute(&self, sql: &str, bindings: &[DbValue]) -> Result<u64> {
        self.raw_execute_with(sql, bindings, QueryExecutionOptions::default())
            .await
    }

    async fn query_records_with(
        &self,
        compiled: &CompiledSql,
        options: QueryExecutionOptions,
    ) -> Result<Vec<DbRecord>> {
        self.raw_query_with(&compiled.sql, &compiled.bindings, options)
            .await
    }

    async fn query_records(&self, compiled: &CompiledSql) -> Result<Vec<DbRecord>> {
        self.query_records_with(compiled, QueryExecutionOptions::default())
            .await
    }

    async fn execute_compiled_with(
        &self,
        compiled: &CompiledSql,
        options: QueryExecutionOptions,
    ) -> Result<u64> {
        self.raw_execute_with(&compiled.sql, &compiled.bindings, options)
            .await
    }

    async fn execute_compiled(&self, compiled: &CompiledSql) -> Result<u64> {
        self.execute_compiled_with(compiled, QueryExecutionOptions::default())
            .await
    }
}

impl DatabaseManager {
    pub fn disabled() -> Self {
        Self {
            state: Arc::new(DatabaseState::Disabled),
        }
    }

    pub async fn from_config(config: &DatabaseConfig) -> Result<Self> {
        Self::from_config_with_observability(config, None).await
    }

    pub(crate) async fn from_config_with_observability(
        config: &DatabaseConfig,
        observability: Option<&ObservabilityConfig>,
    ) -> Result<Self> {
        if config.url.trim().is_empty() {
            return Ok(Self::disabled());
        }

        if !config.url.starts_with("postgres://") && !config.url.starts_with("postgresql://") {
            return Err(Error::message(
                "Foundry database runtime is Postgres-only and requires a postgres:// URL",
            ));
        }

        let pool = PgPoolOptions::new()
            .min_connections(config.min_connections)
            .max_connections(config.max_connections)
            .acquire_timeout(Duration::from_millis(config.acquire_timeout_ms))
            .idle_timeout(Duration::from_secs(config.idle_timeout_seconds))
            .max_lifetime(Duration::from_secs(config.max_lifetime_seconds))
            .connect(&config.url)
            .await
            .map_err(Error::other)?;

        let read_pool = if let Some(ref read_url) = config.read_url {
            if !read_url.trim().is_empty() {
                let rp = PgPoolOptions::new()
                    .min_connections(config.min_connections)
                    .max_connections(config.max_connections)
                    .acquire_timeout(Duration::from_millis(config.acquire_timeout_ms))
                    .idle_timeout(Duration::from_secs(config.idle_timeout_seconds))
                    .max_lifetime(Duration::from_secs(config.max_lifetime_seconds))
                    .connect(read_url)
                    .await
                    .map_err(Error::other)?;
                Some(rp)
            } else {
                None
            }
        } else {
            None
        };

        let sql_log = SqlLogConfig::from_configs(config, observability);

        Ok(Self {
            state: Arc::new(DatabaseState::Ready(DatabaseRuntime {
                pool,
                read_pool,
                adapters: Arc::new(RwLock::new(BTreeMap::new())),
                sql_log,
            })),
        })
    }

    pub fn is_configured(&self) -> bool {
        matches!(self.state.as_ref(), DatabaseState::Ready(_))
    }

    pub fn pool(&self) -> Result<&PgPool> {
        Ok(&self.runtime()?.pool)
    }

    pub fn register_type_adapter(
        &self,
        postgres_type_name: impl Into<String>,
        db_type: DbType,
    ) -> Result<()> {
        let mut adapters =
            write_unpoisoned(&self.runtime()?.adapters, "database type adapter registry");
        adapters.insert(normalize_type_name(&postgres_type_name.into()), db_type);
        Ok(())
    }

    pub fn registered_type_adapter(&self, postgres_type_name: &str) -> Result<Option<DbType>> {
        let adapters = read_unpoisoned(&self.runtime()?.adapters, "database type adapter registry");
        Ok(adapters
            .get(&normalize_type_name(postgres_type_name))
            .copied())
    }

    pub async fn ping(&self) -> Result<()> {
        sqlx::query("SELECT 1")
            .execute(self.pool()?)
            .await
            .map_err(Error::other)?;
        Ok(())
    }

    pub async fn begin(&self) -> Result<DatabaseTransaction> {
        let runtime = self.runtime()?;
        let transaction = runtime.pool.begin().await.map_err(Error::other)?;
        Ok(DatabaseTransaction {
            inner: Mutex::new(Some(transaction)),
            adapters: runtime.adapters.clone(),
            sql_log: runtime.sql_log.clone(),
        })
    }

    pub(crate) async fn acquire_session(&self) -> Result<DatabaseSession> {
        let runtime = self.runtime()?;
        let connection = runtime.pool.acquire().await.map_err(Error::other)?;
        Ok(DatabaseSession {
            pool: runtime.pool.clone(),
            inner: Arc::new(Mutex::new(Some(connection))),
            adapters: runtime.adapters.clone(),
            sql_log: runtime.sql_log.clone(),
        })
    }

    pub async fn raw_query(&self, sql: &str, bindings: &[DbValue]) -> Result<Vec<DbRecord>> {
        <Self as QueryExecutor>::raw_query(self, sql, bindings).await
    }

    pub async fn raw_query_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<Vec<DbRecord>> {
        <Self as QueryExecutor>::raw_query_with(self, sql, bindings, options).await
    }

    pub async fn raw_execute(&self, sql: &str, bindings: &[DbValue]) -> Result<u64> {
        <Self as QueryExecutor>::raw_execute(self, sql, bindings).await
    }

    pub async fn raw_execute_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<u64> {
        <Self as QueryExecutor>::raw_execute_with(self, sql, bindings, options).await
    }

    pub fn raw_stream<'a>(
        &'a self,
        sql: &'a str,
        bindings: &'a [DbValue],
        options: QueryExecutionOptions,
    ) -> DbRecordStream<'a> {
        let compiled = CompiledSql {
            sql: sql.to_string(),
            bindings: bindings.to_vec(),
        };
        <Self as QueryExecutor>::stream_records(self, compiled, options)
    }

    fn runtime(&self) -> Result<&DatabaseRuntime> {
        match self.state.as_ref() {
            DatabaseState::Disabled => Err(Error::message("database is not configured")),
            DatabaseState::Ready(runtime) => Ok(runtime),
        }
    }
}

#[async_trait]
impl QueryExecutor for DatabaseManager {
    async fn raw_query_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<Vec<DbRecord>> {
        let runtime = self.runtime()?;
        let pool = runtime.pool_for_reads(options.use_write_pool);
        let mut connection = pool.acquire().await.map_err(Error::other)?;
        query_records_on_connection(
            connection.as_mut(),
            &runtime.adapters,
            sql,
            bindings,
            &options,
            TimeoutMode::Session,
            &runtime.sql_log,
        )
        .await
    }

    async fn raw_execute_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<u64> {
        let runtime = self.runtime()?;
        let mut connection = runtime.pool.acquire().await.map_err(Error::other)?;
        execute_on_connection(
            connection.as_mut(),
            sql,
            bindings,
            &options,
            TimeoutMode::Session,
            &runtime.sql_log,
        )
        .await
    }

    fn stream_records<'a>(
        &'a self,
        compiled: CompiledSql,
        options: QueryExecutionOptions,
    ) -> DbRecordStream<'a> {
        let runtime = match self.runtime() {
            Ok(runtime) => runtime,
            Err(error) => return single_error_stream(error),
        };

        let pool = runtime.pool_for_reads(options.use_write_pool);
        spawn_native_stream(
            pool.clone(),
            runtime.adapters.clone(),
            compiled,
            options,
            runtime.sql_log.clone(),
        )
    }
}

#[async_trait]
impl QueryExecutor for DatabaseTransaction {
    async fn raw_query_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<Vec<DbRecord>> {
        let mut guard = self.inner.lock().await;
        let transaction = guard
            .as_mut()
            .ok_or_else(|| Error::message("database transaction has already been completed"))?;
        query_records_on_connection(
            transaction.as_mut(),
            &self.adapters,
            sql,
            bindings,
            &options,
            TimeoutMode::Local,
            &self.sql_log,
        )
        .await
    }

    async fn raw_execute_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<u64> {
        let mut guard = self.inner.lock().await;
        let transaction = guard
            .as_mut()
            .ok_or_else(|| Error::message("database transaction has already been completed"))?;
        execute_on_connection(
            transaction.as_mut(),
            sql,
            bindings,
            &options,
            TimeoutMode::Local,
            &self.sql_log,
        )
        .await
    }
}

impl DatabaseTransaction {
    pub async fn raw_query(&self, sql: &str, bindings: &[DbValue]) -> Result<Vec<DbRecord>> {
        <Self as QueryExecutor>::raw_query(self, sql, bindings).await
    }

    pub async fn raw_query_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<Vec<DbRecord>> {
        <Self as QueryExecutor>::raw_query_with(self, sql, bindings, options).await
    }

    pub async fn raw_execute(&self, sql: &str, bindings: &[DbValue]) -> Result<u64> {
        <Self as QueryExecutor>::raw_execute(self, sql, bindings).await
    }

    pub async fn raw_execute_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<u64> {
        <Self as QueryExecutor>::raw_execute_with(self, sql, bindings, options).await
    }

    /// Set a PostgreSQL session configuration value for the current transaction.
    ///
    /// This wraps `set_config(name, value, true)` so domain code can communicate
    /// transaction-scoped context to database triggers without hand-writing SQL.
    pub async fn set_local_config(&self, name: &str, value: &str) -> Result<()> {
        self.raw_execute(
            "SELECT set_config($1, $2, true)",
            &[
                DbValue::Text(name.to_string()),
                DbValue::Text(value.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    pub fn raw_stream<'a>(
        &'a self,
        sql: &'a str,
        bindings: &'a [DbValue],
        options: QueryExecutionOptions,
    ) -> DbRecordStream<'a> {
        let compiled = CompiledSql {
            sql: sql.to_string(),
            bindings: bindings.to_vec(),
        };
        <Self as QueryExecutor>::stream_records(self, compiled, options)
    }

    pub async fn commit(self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        let transaction = guard
            .take()
            .ok_or_else(|| Error::message("database transaction has already been completed"))?;
        transaction.commit().await.map_err(Error::other)
    }

    pub async fn rollback(self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        let transaction = guard
            .take()
            .ok_or_else(|| Error::message("database transaction has already been completed"))?;
        transaction.rollback().await.map_err(Error::other)
    }
}

#[async_trait]
impl QueryExecutor for DatabaseSession {
    async fn raw_query_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<Vec<DbRecord>> {
        let mut connection = self.take_connection().await?;
        let result = query_records_on_connection(
            connection.as_mut(),
            &self.adapters,
            sql,
            bindings,
            &options,
            TimeoutMode::Session,
            &self.sql_log,
        )
        .await;
        self.return_connection(connection).await;
        result
    }

    async fn raw_execute_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<u64> {
        let mut connection = self.take_connection().await?;
        let result = execute_on_connection(
            connection.as_mut(),
            sql,
            bindings,
            &options,
            TimeoutMode::Session,
            &self.sql_log,
        )
        .await;
        self.return_connection(connection).await;
        result
    }

    fn stream_records<'a>(
        &'a self,
        compiled: CompiledSql,
        options: QueryExecutionOptions,
    ) -> DbRecordStream<'a> {
        spawn_session_stream(
            self.pool.clone(),
            self.inner.clone(),
            self.adapters.clone(),
            compiled,
            options,
            self.sql_log.clone(),
        )
    }
}

impl DatabaseSession {
    async fn take_connection(&self) -> Result<PoolConnection<Postgres>> {
        let mut guard = self.inner.lock().await;
        if let Some(connection) = guard.take() {
            return Ok(connection);
        }
        drop(guard);
        self.pool.acquire().await.map_err(Error::other)
    }

    async fn return_connection(&self, connection: PoolConnection<Postgres>) {
        let mut guard = self.inner.lock().await;
        if guard.is_none() {
            *guard = Some(connection);
        }
    }

    pub(crate) async fn begin_transaction(&self) -> Result<()> {
        self.raw_execute("BEGIN", &[]).await?;
        Ok(())
    }

    pub(crate) async fn commit_transaction(&self) -> Result<()> {
        self.raw_execute("COMMIT", &[]).await?;
        Ok(())
    }

    pub(crate) async fn rollback_transaction(&self) -> Result<()> {
        self.raw_execute("ROLLBACK", &[]).await?;
        Ok(())
    }

    pub(crate) async fn try_acquire_advisory_lock(&self, key: i64) -> Result<bool> {
        let rows = self
            .raw_query(
                "SELECT pg_try_advisory_lock($1) AS acquired",
                &[DbValue::Int64(key)],
            )
            .await?;
        rows.first()
            .ok_or_else(|| Error::message("advisory lock query returned no rows"))?
            .decode("acquired")
    }

    pub(crate) async fn release_advisory_lock(&self, key: i64) -> Result<()> {
        self.raw_query(
            "SELECT pg_advisory_unlock($1)::text",
            &[DbValue::Int64(key)],
        )
        .await?;
        Ok(())
    }
}

fn fallback_stream<'a>(
    executor: &'a dyn QueryExecutor,
    compiled: CompiledSql,
    options: QueryExecutionOptions,
) -> DbRecordStream<'a> {
    enum State<'a> {
        Init {
            executor: &'a dyn QueryExecutor,
            compiled: CompiledSql,
            options: QueryExecutionOptions,
        },
        Ready(VecDeque<DbRecord>),
        Done,
    }

    Box::pin(stream::unfold(
        State::Init {
            executor,
            compiled,
            options,
        },
        |state| async move {
            match state {
                State::Init {
                    executor,
                    compiled,
                    options,
                } => match executor.query_records_with(&compiled, options).await {
                    Ok(records) => {
                        let mut queue: VecDeque<_> = records.into();
                        queue
                            .pop_front()
                            .map(|record| (Ok(record), State::Ready(queue)))
                    }
                    Err(error) => Some((Err(error), State::Done)),
                },
                State::Ready(mut queue) => queue
                    .pop_front()
                    .map(|record| (Ok(record), State::Ready(queue))),
                State::Done => None,
            }
        },
    ))
}

fn spawn_native_stream(
    pool: PgPool,
    adapters: Arc<RwLock<BTreeMap<String, DbType>>>,
    compiled: CompiledSql,
    options: QueryExecutionOptions,
    sql_log: SqlLogConfig,
) -> DbRecordStream<'static> {
    let (sender, receiver) = mpsc::channel(16);
    let (cancel_tx, cancel_rx) = oneshot::channel();

    let error_sender = sender.clone();
    let handle = spawn_stream_task(error_sender, async move {
        if sql_log.log_queries {
            tracing::debug!(
                target: "foundry.sql",
                sql = %sql_for_observability(&compiled.sql, sql_log.redact_sql_literals),
                label = ?options.label,
                "stream started"
            );
        }

        let mut cancel_rx = cancel_rx;
        let mut connection = tokio::select! {
            _ = &mut cancel_rx => return Ok(()),
            acquired = pool.acquire() => acquired.map_err(Error::other)?,
        };
        let adapter_snapshot = snapshot_adapters(&adapters)?;
        configure_statement_timeout(connection.as_mut(), &options, TimeoutMode::Session).await?;

        let stream_result = stream_rows_from_connection(
            connection.as_mut(),
            &adapter_snapshot,
            &compiled,
            &options,
            &sender,
            &mut cancel_rx,
        )
        .await;

        let reset_result = reset_statement_timeout(connection.as_mut(), TimeoutMode::Session).await;
        stream_result?;
        reset_result
    });

    receiver_stream(receiver, cancel_tx, handle)
}

fn spawn_session_stream(
    pool: PgPool,
    holder: Arc<Mutex<Option<PoolConnection<Postgres>>>>,
    adapters: Arc<RwLock<BTreeMap<String, DbType>>>,
    compiled: CompiledSql,
    options: QueryExecutionOptions,
    sql_log: SqlLogConfig,
) -> DbRecordStream<'static> {
    let (sender, receiver) = mpsc::channel(16);
    let (cancel_tx, cancel_rx) = oneshot::channel();

    let error_sender = sender.clone();
    let handle = spawn_stream_task(error_sender, async move {
        if sql_log.log_queries {
            tracing::debug!(
                target: "foundry.sql",
                sql = %sql_for_observability(&compiled.sql, sql_log.redact_sql_literals),
                label = ?options.label,
                "stream started"
            );
        }

        let mut cancel_rx = cancel_rx;
        let mut connection = {
            let mut guard = holder.lock().await;
            guard.take()
        };
        let mut connection = match connection.take() {
            Some(connection) => connection,
            None => {
                tokio::select! {
                    _ = &mut cancel_rx => return Ok(()),
                    acquired = pool.acquire() => acquired.map_err(Error::other)?,
                }
            }
        };

        let adapter_snapshot = snapshot_adapters(&adapters)?;
        configure_statement_timeout(connection.as_mut(), &options, TimeoutMode::Session).await?;

        let stream_result = stream_rows_from_connection(
            connection.as_mut(),
            &adapter_snapshot,
            &compiled,
            &options,
            &sender,
            &mut cancel_rx,
        )
        .await;

        let reset_result = reset_statement_timeout(connection.as_mut(), TimeoutMode::Session).await;
        {
            let mut guard = holder.lock().await;
            if guard.is_none() {
                *guard = Some(connection);
            }
        }

        stream_result?;
        reset_result
    });

    receiver_stream(receiver, cancel_tx, handle)
}

async fn query_records_on_connection(
    connection: &mut PgConnection,
    adapters: &Arc<RwLock<BTreeMap<String, DbType>>>,
    sql: &str,
    bindings: &[DbValue],
    options: &QueryExecutionOptions,
    timeout_mode: TimeoutMode,
    sql_log: &SqlLogConfig,
) -> Result<Vec<DbRecord>> {
    log_sql_start(sql_log, sql, bindings, &options.label, "query");
    let start = Instant::now();

    let adapter_snapshot = snapshot_adapters(adapters)?;
    configure_statement_timeout(connection, options, timeout_mode).await?;
    let query = bind_query(sql, bindings)?;
    let rows =
        apply_outer_timeout(query.fetch_all(&mut *connection), options, "query", sql).await?;
    reset_statement_timeout(connection, timeout_mode).await?;
    let result: Result<Vec<DbRecord>> = rows
        .iter()
        .map(|row| decode_row(row, sql, options.label.as_deref(), &adapter_snapshot))
        .collect();

    log_sql_complete(
        sql_log,
        sql,
        start.elapsed(),
        &options.label,
        "query",
        rows.len() as u64,
    );
    result
}

async fn execute_on_connection(
    connection: &mut PgConnection,
    sql: &str,
    bindings: &[DbValue],
    options: &QueryExecutionOptions,
    timeout_mode: TimeoutMode,
    sql_log: &SqlLogConfig,
) -> Result<u64> {
    log_sql_start(sql_log, sql, bindings, &options.label, "execute");
    let start = Instant::now();

    configure_statement_timeout(connection, options, timeout_mode).await?;
    let query = bind_query(sql, bindings)?;
    let result =
        apply_outer_timeout(query.execute(&mut *connection), options, "execution", sql).await?;
    reset_statement_timeout(connection, timeout_mode).await?;
    let rows_affected = result.rows_affected();

    log_sql_complete(
        sql_log,
        sql,
        start.elapsed(),
        &options.label,
        "execute",
        rows_affected,
    );
    Ok(rows_affected)
}

fn log_sql_start(
    sql_log: &SqlLogConfig,
    sql: &str,
    bindings: &[DbValue],
    label: &Option<String>,
    kind: &str,
) {
    if sql_log.log_queries {
        if sql_log.log_query_bindings {
            tracing::debug!(
                target: "foundry.sql",
                sql = %sql_for_observability(sql, sql_log.redact_sql_literals),
                bindings = ?bindings,
                label = ?label,
                kind,
            );
        } else {
            tracing::debug!(
                target: "foundry.sql",
                sql = %sql_for_observability(sql, sql_log.redact_sql_literals),
                binding_count = bindings.len(),
                label = ?label,
                kind,
            );
        }
    }
}

fn log_sql_complete(
    sql_log: &SqlLogConfig,
    sql: &str,
    elapsed: Duration,
    label: &Option<String>,
    kind: &str,
    rows: u64,
) {
    let elapsed_ms = elapsed.as_millis() as u64;
    if sql_log.capture_enabled {
        record_sql_observation(sql, elapsed_ms, label.as_deref(), kind, rows);
    }

    if sql_log.log_queries {
        tracing::debug!(
            target: "foundry.sql",
            duration_ms = elapsed_ms,
            rows,
            label = ?label,
            kind,
            "completed"
        );
    }

    if let Some(threshold) = sql_log.slow_threshold {
        if elapsed > threshold {
            tracing::warn!(
                target: "foundry.sql",
                sql = %sql_for_observability(sql, sql_log.redact_sql_literals),
                duration_ms = elapsed_ms,
                label = ?label,
                "slow query detected"
            );
            record_slow_query(
                sql,
                elapsed_ms,
                label.as_deref(),
                sql_log.slow_query_retention,
                sql_log.redact_sql_literals,
            );
        }
    }
}

async fn stream_rows_from_connection(
    connection: &mut sqlx::postgres::PgConnection,
    adapters: &BTreeMap<String, DbType>,
    compiled: &CompiledSql,
    options: &QueryExecutionOptions,
    sender: &mpsc::Sender<Result<DbRecord>>,
    cancel_rx: &mut oneshot::Receiver<()>,
) -> Result<()> {
    let query = bind_query(&compiled.sql, &compiled.bindings)?;
    let mut rows = query.fetch(connection);

    loop {
        let next_row = if let Some(timeout_duration) = options.timeout {
            tokio::select! {
                _ = &mut *cancel_rx => return Ok(()),
                row = timeout(safety_timeout(timeout_duration), rows.next()) => {
                    row.map_err(|_| outer_timeout_error(options, "stream", &compiled.sql))?
                }
            }
        } else {
            tokio::select! {
                _ = &mut *cancel_rx => return Ok(()),
                row = rows.next() => row,
            }
        };

        match next_row {
            Some(Ok(row)) => {
                let record = decode_row(&row, &compiled.sql, options.label.as_deref(), adapters)?;
                if sender.send(Ok(record)).await.is_err() {
                    break;
                }
            }
            Some(Err(error)) => {
                let mapped = map_sqlx_operation_error(error, options, "stream", &compiled.sql);
                let _ = sender.send(Err(mapped)).await;
                break;
            }
            None => break,
        }
    }

    Ok(())
}

async fn configure_statement_timeout(
    connection: &mut PgConnection,
    options: &QueryExecutionOptions,
    mode: TimeoutMode,
) -> Result<()> {
    let timeout_value = options
        .timeout
        .map(timeout_millis)
        .unwrap_or_else(|| "0".to_string());
    let local = matches!(mode, TimeoutMode::Local);

    sqlx::query("SELECT set_config('statement_timeout', $1, $2)")
        .bind(timeout_value)
        .bind(local)
        .execute(connection)
        .await
        .map_err(Error::other)?;
    Ok(())
}

async fn reset_statement_timeout(connection: &mut PgConnection, mode: TimeoutMode) -> Result<()> {
    sqlx::query("SELECT set_config('statement_timeout', '0', $1)")
        .bind(matches!(mode, TimeoutMode::Local))
        .execute(connection)
        .await
        .map_err(Error::other)?;
    Ok(())
}

fn snapshot_adapters(
    adapters: &Arc<RwLock<BTreeMap<String, DbType>>>,
) -> Result<BTreeMap<String, DbType>> {
    Ok(read_unpoisoned(adapters, "database type adapter registry").clone())
}

async fn apply_outer_timeout<F, T>(
    future: F,
    options: &QueryExecutionOptions,
    action: &str,
    sql: &str,
) -> Result<T>
where
    F: std::future::Future<Output = std::result::Result<T, sqlx::Error>>,
{
    if let Some(timeout_duration) = options.timeout {
        timeout(safety_timeout(timeout_duration), future)
            .await
            .map_err(|_| outer_timeout_error(options, action, sql))?
            .map_err(|error| map_sqlx_operation_error(error, options, action, sql))
    } else {
        future
            .await
            .map_err(|error| map_sqlx_operation_error(error, options, action, sql))
    }
}

fn spawn_stream_task<Fut>(
    error_sender: mpsc::Sender<Result<DbRecord>>,
    future: Fut,
) -> JoinHandle<()>
where
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    tokio::spawn(async move {
        match catch_future_panic(future).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let _ = error_sender.send(Err(error)).await;
            }
            Err(panic) => {
                let _ = error_sender
                    .send(Err(database_stream_panic_error(panic)))
                    .await;
            }
        }
    })
}

fn receiver_stream(
    receiver: mpsc::Receiver<Result<DbRecord>>,
    cancel: oneshot::Sender<()>,
    handle: JoinHandle<()>,
) -> DbRecordStream<'static> {
    Box::pin(SpawnedDbRecordStream::new(receiver, cancel, handle))
}

struct SpawnedDbRecordStream {
    receiver: mpsc::Receiver<Result<DbRecord>>,
    cancel: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl SpawnedDbRecordStream {
    fn new(
        receiver: mpsc::Receiver<Result<DbRecord>>,
        cancel: oneshot::Sender<()>,
        handle: JoinHandle<()>,
    ) -> Self {
        Self {
            receiver,
            cancel: Some(cancel),
            handle: Some(handle),
        }
    }
}

impl futures_util::Stream for SpawnedDbRecordStream {
    type Item = Result<DbRecord>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.receiver).poll_recv(cx)
    }
}

impl Drop for SpawnedDbRecordStream {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        let _ = self.handle.take();
    }
}

fn single_error_stream<'a>(error: Error) -> DbRecordStream<'a> {
    Box::pin(stream::once(async move { Err(error) }))
}

fn database_stream_panic_error(panic: Box<dyn std::any::Any + Send>) -> Error {
    Error::message(format!(
        "database stream panicked: {}",
        panic_payload_message(panic)
    ))
}

fn outer_timeout_error(options: &QueryExecutionOptions, action: &str, sql: &str) -> Error {
    let duration = options
        .timeout
        .map(|timeout| timeout.as_millis().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    Error::message(format!(
        "database {action} timed out after {duration}ms{} while running `{sql}`",
        label_suffix(options)
    ))
}

fn map_sqlx_operation_error(
    error: sqlx::Error,
    options: &QueryExecutionOptions,
    action: &str,
    sql: &str,
) -> Error {
    if is_statement_timeout(&error) {
        return Error::message(format!(
            "database {action} timed out after {}ms{} while running `{sql}`: {}",
            options
                .timeout
                .map(|timeout| timeout.as_millis())
                .unwrap_or_default(),
            label_suffix(options),
            error
        ));
    }

    Error::message(format!(
        "database {action} failed{} while running `{sql}`: {error}",
        label_suffix(options)
    ))
}

fn is_statement_timeout(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(database_error) => {
            database_error.code().as_deref() == Some("57014")
                && database_error
                    .message()
                    .to_ascii_lowercase()
                    .contains("statement timeout")
        }
        _ => false,
    }
}

fn label_suffix(options: &QueryExecutionOptions) -> String {
    options
        .label
        .as_ref()
        .map(|label| format!(" for `{label}`"))
        .unwrap_or_default()
}

fn safety_timeout(timeout_duration: Duration) -> Duration {
    timeout_duration.saturating_add(Duration::from_millis(50))
}

fn timeout_millis(timeout_duration: Duration) -> String {
    timeout_duration.as_millis().to_string()
}

#[derive(Clone, Copy)]
enum TimeoutMode {
    Session,
    Local,
}

fn normalize_type_name(type_name: &str) -> String {
    let normalized = type_name.trim().replace('"', "").to_ascii_lowercase();
    match normalized.strip_suffix("[]") {
        Some(element_type) => format!("_{}", element_type.trim()),
        None => normalized,
    }
}

fn bind_query<'q>(
    sql: &'q str,
    bindings: &'q [DbValue],
) -> Result<sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments>> {
    let mut query = sqlx::query(sql);
    for binding in bindings {
        query = match binding {
            DbValue::Null(db_type) => bind_null(query, *db_type),
            DbValue::Int16(value) => query.bind(*value),
            DbValue::Int32(value) => query.bind(*value),
            DbValue::Int64(value) => query.bind(*value),
            DbValue::Bool(value) => query.bind(*value),
            DbValue::Float32(value) => query.bind(*value),
            DbValue::Float64(value) => query.bind(*value),
            DbValue::Numeric(value) => query.bind(value.to_string()),
            DbValue::Text(value) => query.bind(value.clone()),
            DbValue::Json(value) => query.bind(sqlx::types::Json(value.clone())),
            DbValue::Uuid(value) => query.bind(*value),
            DbValue::TimestampTz(value) => query.bind(value.as_chrono()),
            DbValue::Timestamp(value) => query.bind(value.as_chrono()),
            DbValue::Date(value) => query.bind(value.as_chrono()),
            DbValue::Time(value) => query.bind(value.as_chrono()),
            DbValue::Bytea(value) => query.bind(value.clone()),
            DbValue::Int16Array(value) => query.bind(value.clone()),
            DbValue::Int32Array(value) => query.bind(value.clone()),
            DbValue::Int64Array(value) => query.bind(value.clone()),
            DbValue::BoolArray(value) => query.bind(value.clone()),
            DbValue::Float32Array(value) => query.bind(value.clone()),
            DbValue::Float64Array(value) => query.bind(value.clone()),
            DbValue::NumericArray(value) => {
                query.bind(value.iter().map(ToString::to_string).collect::<Vec<_>>())
            }
            DbValue::TextArray(value) => query.bind(value.clone()),
            DbValue::JsonArray(value) => query.bind(
                value
                    .iter()
                    .cloned()
                    .map(sqlx::types::Json)
                    .collect::<Vec<_>>(),
            ),
            DbValue::UuidArray(value) => query.bind(value.clone()),
            DbValue::TimestampTzArray(value) => {
                query.bind(value.iter().map(DateTime::as_chrono).collect::<Vec<_>>())
            }
            DbValue::TimestampArray(value) => query.bind(
                value
                    .iter()
                    .map(LocalDateTime::as_chrono)
                    .collect::<Vec<_>>(),
            ),
            DbValue::DateArray(value) => {
                query.bind(value.iter().map(Date::as_chrono).collect::<Vec<_>>())
            }
            DbValue::TimeArray(value) => {
                query.bind(value.iter().map(Time::as_chrono).collect::<Vec<_>>())
            }
            DbValue::ByteaArray(value) => query.bind(value.clone()),
        };
    }
    Ok(query)
}

fn bind_null<'q>(
    query: sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments>,
    db_type: DbType,
) -> sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments> {
    match db_type {
        DbType::Int16 => query.bind(Option::<i16>::None),
        DbType::Int32 => query.bind(Option::<i32>::None),
        DbType::Int64 => query.bind(Option::<i64>::None),
        DbType::Bool => query.bind(Option::<bool>::None),
        DbType::Float32 => query.bind(Option::<f32>::None),
        DbType::Float64 => query.bind(Option::<f64>::None),
        DbType::Numeric => query.bind(Option::<String>::None),
        DbType::Text => query.bind(Option::<String>::None),
        DbType::Json => query.bind(Option::<sqlx::types::Json<serde_json::Value>>::None),
        DbType::Uuid => query.bind(Option::<Uuid>::None),
        DbType::TimestampTz => query.bind(Option::<ChronoDateTime<ChronoUtc>>::None),
        DbType::Timestamp => query.bind(Option::<ChronoNaiveDateTime>::None),
        DbType::Date => query.bind(Option::<ChronoDate>::None),
        DbType::Time => query.bind(Option::<ChronoTime>::None),
        DbType::Bytea => query.bind(Option::<Vec<u8>>::None),
        DbType::Int16Array => query.bind(Option::<Vec<i16>>::None),
        DbType::Int32Array => query.bind(Option::<Vec<i32>>::None),
        DbType::Int64Array => query.bind(Option::<Vec<i64>>::None),
        DbType::BoolArray => query.bind(Option::<Vec<bool>>::None),
        DbType::Float32Array => query.bind(Option::<Vec<f32>>::None),
        DbType::Float64Array => query.bind(Option::<Vec<f64>>::None),
        DbType::NumericArray => query.bind(Option::<Vec<String>>::None),
        DbType::TextArray => query.bind(Option::<Vec<String>>::None),
        DbType::JsonArray => query.bind(Option::<Vec<sqlx::types::Json<serde_json::Value>>>::None),
        DbType::UuidArray => query.bind(Option::<Vec<Uuid>>::None),
        DbType::TimestampTzArray => query.bind(Option::<Vec<ChronoDateTime<ChronoUtc>>>::None),
        DbType::TimestampArray => query.bind(Option::<Vec<ChronoNaiveDateTime>>::None),
        DbType::DateArray => query.bind(Option::<Vec<ChronoDate>>::None),
        DbType::TimeArray => query.bind(Option::<Vec<ChronoTime>>::None),
        DbType::ByteaArray => query.bind(Option::<Vec<Vec<u8>>>::None),
    }
}

fn decode_row(
    row: &PgRow,
    sql: &str,
    label: Option<&str>,
    adapters: &BTreeMap<String, DbType>,
) -> Result<DbRecord> {
    let mut record = DbRecord::new();

    for column in row.columns() {
        let name = column.name();
        let value = decode_column(row, name, column.type_info().name(), sql, label, adapters)?;
        record.insert(name.to_string(), value);
    }

    Ok(record)
}

fn decode_column(
    row: &PgRow,
    name: &str,
    type_name: &str,
    sql: &str,
    label: Option<&str>,
    adapters: &BTreeMap<String, DbType>,
) -> Result<DbValue> {
    let normalized = normalize_type_name(type_name);
    let mapped = match normalized.as_str() {
        "int2" => Some(DbType::Int16),
        "int4" => Some(DbType::Int32),
        "int8" => Some(DbType::Int64),
        "bool" => Some(DbType::Bool),
        "float4" => Some(DbType::Float32),
        "float8" => Some(DbType::Float64),
        "numeric" => Some(DbType::Numeric),
        "text" | "varchar" | "bpchar" | "char" | "name" => Some(DbType::Text),
        "json" | "jsonb" => Some(DbType::Json),
        "uuid" => Some(DbType::Uuid),
        "timestamptz" => Some(DbType::TimestampTz),
        "timestamp" => Some(DbType::Timestamp),
        "date" => Some(DbType::Date),
        "time" | "timetz" => Some(DbType::Time),
        "bytea" => Some(DbType::Bytea),
        "_int2" => Some(DbType::Int16Array),
        "_int4" => Some(DbType::Int32Array),
        "_int8" => Some(DbType::Int64Array),
        "_bool" => Some(DbType::BoolArray),
        "_float4" => Some(DbType::Float32Array),
        "_float8" => Some(DbType::Float64Array),
        "_numeric" => Some(DbType::NumericArray),
        "_text" | "_varchar" | "_bpchar" | "_char" | "_name" => Some(DbType::TextArray),
        "_json" | "_jsonb" => Some(DbType::JsonArray),
        "_uuid" => Some(DbType::UuidArray),
        "_timestamptz" => Some(DbType::TimestampTzArray),
        "_timestamp" => Some(DbType::TimestampArray),
        "_date" => Some(DbType::DateArray),
        "_time" | "_timetz" => Some(DbType::TimeArray),
        "_bytea" => Some(DbType::ByteaArray),
        _ => adapters.get(&normalized).copied(),
    }
    .ok_or_else(|| unsupported_type_error(name, type_name, &normalized, sql, label))?;

    decode_column_as(row, name, mapped).map_err(|error| {
        Error::message(format!(
            "failed to decode column `{name}` with postgres type `{type_name}`{}: {error}",
            format_query_context(sql, label)
        ))
    })
}

fn decode_column_as(row: &PgRow, name: &str, db_type: DbType) -> Result<DbValue> {
    match db_type {
        DbType::Int16 => row
            .try_get::<Option<i16>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::Int16)
                    .unwrap_or(DbValue::Null(DbType::Int16))
            })
            .map_err(Error::other),
        DbType::Int32 => row
            .try_get::<Option<i32>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::Int32)
                    .unwrap_or(DbValue::Null(DbType::Int32))
            })
            .map_err(Error::other),
        DbType::Int64 => row
            .try_get::<Option<i64>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::Int64)
                    .unwrap_or(DbValue::Null(DbType::Int64))
            })
            .map_err(Error::other),
        DbType::Bool => row
            .try_get::<Option<bool>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::Bool)
                    .unwrap_or(DbValue::Null(DbType::Bool))
            })
            .map_err(Error::other),
        DbType::Float32 => row
            .try_get::<Option<f32>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::Float32)
                    .unwrap_or(DbValue::Null(DbType::Float32))
            })
            .map_err(Error::other),
        DbType::Float64 => row
            .try_get::<Option<f64>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::Float64)
                    .unwrap_or(DbValue::Null(DbType::Float64))
            })
            .map_err(Error::other),
        DbType::Numeric => row
            .try_get::<Option<BigDecimal>, _>(name)
            .map(|value| match value {
                Some(value) => decode_numeric_value(value).map(DbValue::Numeric),
                None => Ok(DbValue::Null(DbType::Numeric)),
            })
            .map_err(Error::other)?
            .map_err(Error::other),
        DbType::Text => row
            .try_get::<Option<String>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::Text)
                    .unwrap_or(DbValue::Null(DbType::Text))
            })
            .map_err(Error::other),
        DbType::Json => row
            .try_get::<Option<sqlx::types::Json<serde_json::Value>>, _>(name)
            .map(|value| {
                value
                    .map(|value| DbValue::Json(value.0))
                    .unwrap_or(DbValue::Null(DbType::Json))
            })
            .map_err(Error::other),
        DbType::Uuid => row
            .try_get::<Option<Uuid>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::Uuid)
                    .unwrap_or(DbValue::Null(DbType::Uuid))
            })
            .map_err(Error::other),
        DbType::TimestampTz => row
            .try_get::<Option<ChronoDateTime<ChronoUtc>>, _>(name)
            .map(|value| {
                value
                    .map(DateTime::from_chrono)
                    .map(DbValue::TimestampTz)
                    .unwrap_or(DbValue::Null(DbType::TimestampTz))
            })
            .map_err(Error::other),
        DbType::Timestamp => row
            .try_get::<Option<ChronoNaiveDateTime>, _>(name)
            .map(|value| {
                value
                    .map(LocalDateTime::from_chrono)
                    .map(DbValue::Timestamp)
                    .unwrap_or(DbValue::Null(DbType::Timestamp))
            })
            .map_err(Error::other),
        DbType::Date => row
            .try_get::<Option<ChronoDate>, _>(name)
            .map(|value| {
                value
                    .map(Date::from_chrono)
                    .map(DbValue::Date)
                    .unwrap_or(DbValue::Null(DbType::Date))
            })
            .map_err(Error::other),
        DbType::Time => row
            .try_get::<Option<ChronoTime>, _>(name)
            .map(|value| {
                value
                    .map(Time::from_chrono)
                    .map(DbValue::Time)
                    .unwrap_or(DbValue::Null(DbType::Time))
            })
            .map_err(Error::other),
        DbType::Bytea => row
            .try_get::<Option<Vec<u8>>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::Bytea)
                    .unwrap_or(DbValue::Null(DbType::Bytea))
            })
            .map_err(Error::other),
        DbType::Int16Array => row
            .try_get::<Option<Vec<i16>>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::Int16Array)
                    .unwrap_or(DbValue::Null(DbType::Int16Array))
            })
            .map_err(Error::other),
        DbType::Int32Array => row
            .try_get::<Option<Vec<i32>>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::Int32Array)
                    .unwrap_or(DbValue::Null(DbType::Int32Array))
            })
            .map_err(Error::other),
        DbType::Int64Array => row
            .try_get::<Option<Vec<i64>>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::Int64Array)
                    .unwrap_or(DbValue::Null(DbType::Int64Array))
            })
            .map_err(Error::other),
        DbType::BoolArray => row
            .try_get::<Option<Vec<bool>>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::BoolArray)
                    .unwrap_or(DbValue::Null(DbType::BoolArray))
            })
            .map_err(Error::other),
        DbType::Float32Array => row
            .try_get::<Option<Vec<f32>>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::Float32Array)
                    .unwrap_or(DbValue::Null(DbType::Float32Array))
            })
            .map_err(Error::other),
        DbType::Float64Array => row
            .try_get::<Option<Vec<f64>>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::Float64Array)
                    .unwrap_or(DbValue::Null(DbType::Float64Array))
            })
            .map_err(Error::other),
        DbType::NumericArray => row
            .try_get::<Option<Vec<BigDecimal>>, _>(name)
            .map(|value| match value {
                Some(values) => decode_numeric_values(values).map(DbValue::NumericArray),
                None => Ok(DbValue::Null(DbType::NumericArray)),
            })
            .map_err(Error::other)?
            .map_err(Error::other),
        DbType::TextArray => row
            .try_get::<Option<Vec<String>>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::TextArray)
                    .unwrap_or(DbValue::Null(DbType::TextArray))
            })
            .map_err(Error::other),
        DbType::JsonArray => row
            .try_get::<Option<Vec<sqlx::types::Json<serde_json::Value>>>, _>(name)
            .map(|value| match value {
                Some(values) => {
                    DbValue::JsonArray(values.into_iter().map(|value| value.0).collect())
                }
                None => DbValue::Null(DbType::JsonArray),
            })
            .map_err(Error::other),
        DbType::UuidArray => row
            .try_get::<Option<Vec<Uuid>>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::UuidArray)
                    .unwrap_or(DbValue::Null(DbType::UuidArray))
            })
            .map_err(Error::other),
        DbType::TimestampTzArray => row
            .try_get::<Option<Vec<ChronoDateTime<ChronoUtc>>>, _>(name)
            .map(|value| {
                value
                    .map(|values| {
                        DbValue::TimestampTzArray(
                            values.into_iter().map(DateTime::from_chrono).collect(),
                        )
                    })
                    .unwrap_or(DbValue::Null(DbType::TimestampTzArray))
            })
            .map_err(Error::other),
        DbType::TimestampArray => row
            .try_get::<Option<Vec<ChronoNaiveDateTime>>, _>(name)
            .map(|value| {
                value
                    .map(|values| {
                        DbValue::TimestampArray(
                            values.into_iter().map(LocalDateTime::from_chrono).collect(),
                        )
                    })
                    .unwrap_or(DbValue::Null(DbType::TimestampArray))
            })
            .map_err(Error::other),
        DbType::DateArray => row
            .try_get::<Option<Vec<ChronoDate>>, _>(name)
            .map(|value| {
                value
                    .map(|values| {
                        DbValue::DateArray(values.into_iter().map(Date::from_chrono).collect())
                    })
                    .unwrap_or(DbValue::Null(DbType::DateArray))
            })
            .map_err(Error::other),
        DbType::TimeArray => row
            .try_get::<Option<Vec<ChronoTime>>, _>(name)
            .map(|value| {
                value
                    .map(|values| {
                        DbValue::TimeArray(values.into_iter().map(Time::from_chrono).collect())
                    })
                    .unwrap_or(DbValue::Null(DbType::TimeArray))
            })
            .map_err(Error::other),
        DbType::ByteaArray => row
            .try_get::<Option<Vec<Vec<u8>>>, _>(name)
            .map(|value| {
                value
                    .map(DbValue::ByteaArray)
                    .unwrap_or(DbValue::Null(DbType::ByteaArray))
            })
            .map_err(Error::other),
    }
}

fn decode_numeric_value(value: BigDecimal) -> Result<Numeric> {
    Numeric::new(value.to_string())
}

fn decode_numeric_values(values: Vec<BigDecimal>) -> Result<Vec<Numeric>> {
    values.into_iter().map(decode_numeric_value).collect()
}

fn unsupported_type_error(
    name: &str,
    type_name: &str,
    normalized_type: &str,
    sql: &str,
    label: Option<&str>,
) -> Error {
    Error::message(format!(
        "unsupported postgres type `{type_name}` (normalized lookup `{normalized_type}`) for column `{name}`{}; register a database type adapter or add first-class support",
        format_query_context(sql, label)
    ))
}

fn format_query_context(sql: &str, label: Option<&str>) -> String {
    let mut suffix = String::new();
    if let Some(label) = label {
        suffix.push_str(&format!(" in `{label}`"));
    }
    suffix.push_str(&format!(" while running `{sql}`"));
    suffix
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock};

    use futures_util::StreamExt;
    use tokio::sync::{mpsc, oneshot, Mutex};

    use crate::logging::{catch_future_panic, catch_sync_panic};
    use crate::support::sync::lock_unpoisoned;

    use super::{
        n_plus_one_log, normalize_type_name, receiver_stream, recent_n_plus_one_suspects,
        recent_slow_queries, record_slow_query, record_sql_observation, redact_sql_literals,
        scope_http_sql_query_trace, slow_query_log, spawn_stream_task, sql_fingerprint,
        sql_observability_snapshot, DbRecord, DbValue, Result, SlowQueryEntry,
    };

    fn sql_observability_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn normalize_type_name_maps_array_aliases_to_internal_lookup_keys() {
        assert_eq!(normalize_type_name("_text"), "_text");
        assert_eq!(normalize_type_name("text[]"), "_text");
        assert_eq!(normalize_type_name("TEXT[]"), "_text");
        assert_eq!(normalize_type_name("\"TEXT\"[]"), "_text");
        assert_eq!(normalize_type_name("char[]"), "_char");
        assert_eq!(normalize_type_name("CHAR[]"), "_char");
        assert_eq!(normalize_type_name("\"CHAR\"[]"), "_char");
    }

    #[test]
    fn db_record_try_text_helpers_report_missing_and_wrong_types() {
        let mut record = DbRecord::new();
        record.insert("name", DbValue::Text("Foundry".to_string()));
        record.insert("id", DbValue::Uuid(uuid::Uuid::nil()));
        record.insert("count", DbValue::Int64(7));

        assert_eq!(record.try_text("name").unwrap(), "Foundry");
        assert_eq!(
            record.try_text_or_uuid("id").unwrap(),
            uuid::Uuid::nil().to_string()
        );
        assert!(record
            .try_text("missing")
            .unwrap_err()
            .to_string()
            .contains("missing"));
        assert!(record
            .try_text("count")
            .unwrap_err()
            .to_string()
            .contains("expected text"));
    }

    #[test]
    fn slow_query_log_recovers_after_poison() {
        let _guard = sql_observability_test_lock().blocking_lock();
        lock_unpoisoned(slow_query_log(), "slow query log test setup").clear();

        let result = catch_sync_panic(|| {
            let mut log = match slow_query_log().lock() {
                Ok(log) => log,
                Err(poisoned) => poisoned.into_inner(),
            };
            log.push_back(SlowQueryEntry {
                sql: "SELECT before panic".to_string(),
                duration_ms: 1,
                label: None,
                request_id: None,
                trace_id: None,
                recorded_at: "before".to_string(),
            });
            panic!("poison slow query log");
        });
        assert!(result.is_err());

        record_slow_query("SELECT after panic", 42, Some("poison-recovery"), 100, true);

        let queries = recent_slow_queries();
        assert!(queries.iter().any(|query| query.sql == "SELECT after panic"
            && query.duration_ms == 42
            && query.label.as_deref() == Some("poison-recovery")));

        *lock_unpoisoned(slow_query_log(), "slow query log test cleanup") = VecDeque::new();
    }

    #[test]
    fn sql_fingerprint_normalizes_bindings_literals_and_whitespace() {
        assert_eq!(
            sql_fingerprint(
                r#"SELECT  * FROM "users" WHERE id = $1 AND email = 'USER@example.com' LIMIT 10"#
            ),
            r#"select * from "users" where id = ? and email = ? limit ?"#
        );
        assert_eq!(
            sql_fingerprint("select * from api_v2_users where token = ? and score > 42.5"),
            "select * from api_v2_users where token = ? and score > ?"
        );
    }

    #[test]
    fn sql_observability_redacts_literals_and_comments() {
        let sql = "SELECT * FROM users WHERE email = 'secret@example.com' AND score > 42 -- api token\n/* password */ AND note = E'quoted'";

        let redacted = redact_sql_literals(sql);

        assert!(!redacted.contains("secret@example.com"));
        assert!(!redacted.contains("api token"));
        assert!(!redacted.contains("password"));
        assert!(!redacted.contains("quoted"));
        assert!(redacted.contains("email = ?"));
        assert!(redacted.contains("score > ?"));
        assert!(redacted.contains("-- redacted"));
        assert!(redacted.contains("/* redacted */"));
    }

    #[test]
    fn sql_observability_redacts_dollar_quoted_literals_without_eating_plain_tokens() {
        let sql = "SELECT $$secret$$, $tag$hidden$tag$, price_$suffix FROM plans";

        let redacted = redact_sql_literals(sql);

        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("hidden"));
        assert!(redacted.contains("?, ?"));
        assert!(redacted.contains("price_$suffix"));
    }

    #[test]
    fn slow_query_retention_redacts_sql_by_default() {
        let _guard = sql_observability_test_lock().blocking_lock();
        lock_unpoisoned(slow_query_log(), "slow query redaction setup").clear();

        record_slow_query(
            "SELECT * FROM users WHERE token = 'secret-token'",
            900,
            Some("secret.lookup"),
            100,
            true,
        );

        let queries = recent_slow_queries();
        assert_eq!(queries.len(), 1);
        assert!(!queries[0].sql.contains("secret-token"));
        assert_eq!(queries[0].sql, "SELECT * FROM users WHERE token = ?");

        lock_unpoisoned(slow_query_log(), "slow query redaction cleanup").clear();
    }

    #[test]
    fn sql_observability_snapshot_summarizes_and_ranks_slow_queries() {
        let _guard = sql_observability_test_lock().blocking_lock();
        lock_unpoisoned(slow_query_log(), "slow query snapshot setup").clear();
        lock_unpoisoned(n_plus_one_log(), "n+1 snapshot setup").clear();

        record_slow_query("SELECT slow", 750, Some("slow"), 100, true);
        record_slow_query("SELECT slower", 1_200, None, 100, true);

        let snapshot = sql_observability_snapshot(500, 100);
        assert_eq!(snapshot.stats.retained_count, 2);
        assert_eq!(snapshot.stats.capacity, 100);
        assert_eq!(snapshot.stats.slow_query_threshold_ms, 500);
        assert_eq!(snapshot.stats.max_duration_ms, Some(1_200));
        assert_eq!(snapshot.stats.avg_duration_ms, Some(975));
        assert!(snapshot.stats.latest_recorded_at.is_some());
        assert_eq!(snapshot.top_slowest[0].sql, "SELECT slower");
        assert_eq!(snapshot.top_slowest[1].sql, "SELECT slow");

        lock_unpoisoned(slow_query_log(), "slow query snapshot cleanup").clear();
    }

    #[test]
    fn sql_observability_snapshot_respects_slow_query_retention() {
        let _guard = sql_observability_test_lock().blocking_lock();
        lock_unpoisoned(slow_query_log(), "slow query retention setup").clear();
        lock_unpoisoned(n_plus_one_log(), "n+1 retention setup").clear();

        record_slow_query("SELECT first", 100, None, 10, true);
        record_slow_query("SELECT second", 200, None, 10, true);

        let snapshot = sql_observability_snapshot(500, 1);
        assert_eq!(snapshot.stats.retained_count, 1);
        assert_eq!(snapshot.stats.capacity, 1);
        assert_eq!(snapshot.slow_queries[0].sql, "SELECT second");

        let disabled = sql_observability_snapshot(500, 0);
        assert_eq!(disabled.stats.retained_count, 0);
        assert!(disabled.slow_queries.is_empty());

        lock_unpoisoned(slow_query_log(), "slow query retention cleanup").clear();
    }

    #[tokio::test]
    async fn sql_observability_attaches_current_trace_context() {
        use crate::config::DatabaseConfig;
        use crate::logging::TraceContext;

        let _guard = sql_observability_test_lock().lock().await;
        lock_unpoisoned(slow_query_log(), "slow query trace setup").clear();
        lock_unpoisoned(n_plus_one_log(), "n+1 trace setup").clear();

        let config = DatabaseConfig {
            n_plus_one_min_repeats: 2,
            n_plus_one_retention: 5,
            ..DatabaseConfig::default()
        };
        crate::logging::scope_current_trace(
            TraceContext::http("req-sql-trace".to_string()),
            async {
                record_slow_query("SELECT trace slow", 900, Some("trace.slow"), 100, true);
                scope_http_sql_query_trace(
                    Some(config),
                    None,
                    "GET".to_string(),
                    "/trace".to_string(),
                    None,
                    async {
                        for _ in 0..2 {
                            record_sql_observation(
                                "SELECT * FROM users WHERE id = $1",
                                15,
                                Some("trace.lookup"),
                                "query",
                                1,
                            );
                        }
                    },
                )
                .await;
            },
        )
        .await;

        let slow_queries = recent_slow_queries();
        let slow_query = slow_queries
            .iter()
            .find(|query| query.sql == "SELECT trace slow")
            .expect("expected traced slow query");
        assert_eq!(slow_query.request_id.as_deref(), Some("req-sql-trace"));
        assert_eq!(slow_query.trace_id.as_deref(), Some("req-sql-trace"));

        let suspects = recent_n_plus_one_suspects();
        assert_eq!(suspects.len(), 1);
        assert_eq!(suspects[0].request_id.as_deref(), Some("req-sql-trace"));
        assert_eq!(suspects[0].trace_id.as_deref(), Some("req-sql-trace"));

        lock_unpoisoned(slow_query_log(), "slow query trace cleanup").clear();
        lock_unpoisoned(n_plus_one_log(), "n+1 trace cleanup").clear();
    }

    #[tokio::test]
    async fn http_query_trace_records_n_plus_one_suspects_at_threshold() {
        use crate::config::DatabaseConfig;

        let _guard = sql_observability_test_lock().lock().await;
        lock_unpoisoned(n_plus_one_log(), "n+1 threshold setup").clear();

        let config = DatabaseConfig {
            n_plus_one_min_repeats: 3,
            n_plus_one_retention: 5,
            ..DatabaseConfig::default()
        };
        scope_http_sql_query_trace(
            Some(config),
            None,
            "GET".to_string(),
            "/users".to_string(),
            Some("req-n-plus-one".to_string()),
            async {
                for _ in 0..3 {
                    record_sql_observation(
                        "SELECT * FROM users WHERE email = 'secret@example.com'",
                        12,
                        Some("user.lookup"),
                        "query",
                        1,
                    );
                }
            },
        )
        .await;

        let suspects = recent_n_plus_one_suspects();
        assert_eq!(suspects.len(), 1);
        let suspect = &suspects[0];
        assert_eq!(suspect.method, "GET");
        assert_eq!(suspect.path, "/users");
        assert_eq!(suspect.request_id.as_deref(), Some("req-n-plus-one"));
        assert_eq!(suspect.repeat_count, 3);
        assert_eq!(suspect.total_duration_ms, 36);
        assert_eq!(suspect.max_duration_ms, 12);
        assert_eq!(suspect.avg_duration_ms, 12);
        assert_eq!(suspect.rows_total, 3);
        assert_eq!(suspect.labels, vec!["user.lookup"]);
        assert_eq!(suspect.kinds, vec!["query"]);
        assert_eq!(suspect.fingerprint, "select * from users where email = ?");
        assert_eq!(suspect.sample_sql, "SELECT * FROM users WHERE email = ?");

        lock_unpoisoned(n_plus_one_log(), "n+1 threshold cleanup").clear();
    }

    #[tokio::test]
    async fn http_query_trace_respects_observability_capture_switch() {
        use crate::config::{DatabaseConfig, ObservabilityConfig};

        let _guard = sql_observability_test_lock().lock().await;
        lock_unpoisoned(n_plus_one_log(), "n+1 capture setup").clear();

        let database = DatabaseConfig {
            n_plus_one_min_repeats: 2,
            n_plus_one_retention: 5,
            ..DatabaseConfig::default()
        };
        let observability = ObservabilityConfig {
            capture_enabled: false,
            ..ObservabilityConfig::default()
        };
        scope_http_sql_query_trace(
            Some(database),
            Some(observability),
            "GET".to_string(),
            "/users".to_string(),
            Some("req-capture-disabled".to_string()),
            async {
                for _ in 0..2 {
                    record_sql_observation(
                        "SELECT * FROM users WHERE id = $1",
                        12,
                        Some("user.lookup"),
                        "query",
                        1,
                    );
                }
            },
        )
        .await;

        assert!(recent_n_plus_one_suspects().is_empty());
    }

    #[tokio::test]
    async fn query_trace_flushes_when_http_scope_panics() {
        use crate::config::DatabaseConfig;

        let _guard = sql_observability_test_lock().lock().await;
        lock_unpoisoned(n_plus_one_log(), "n+1 panic setup").clear();

        let config = DatabaseConfig {
            n_plus_one_min_repeats: 2,
            n_plus_one_retention: 5,
            ..DatabaseConfig::default()
        };
        let result: std::result::Result<(), _> = catch_future_panic(scope_http_sql_query_trace(
            Some(config),
            None,
            "GET".to_string(),
            "/panic".to_string(),
            Some("req-panic".to_string()),
            async {
                for _ in 0..2 {
                    record_sql_observation(
                        "SELECT * FROM users WHERE id = $1",
                        20,
                        Some("user.lookup"),
                        "query",
                        1,
                    );
                }
                panic!("request exploded");
            },
        ))
        .await;

        assert!(result.is_err());
        let suspects = recent_n_plus_one_suspects();
        assert_eq!(suspects.len(), 1);
        assert_eq!(suspects[0].path, "/panic");
        assert_eq!(suspects[0].repeat_count, 2);

        lock_unpoisoned(n_plus_one_log(), "n+1 panic cleanup").clear();
    }

    #[tokio::test]
    async fn query_trace_ignores_repeats_below_threshold() {
        use crate::config::DatabaseConfig;

        let _guard = sql_observability_test_lock().lock().await;
        lock_unpoisoned(n_plus_one_log(), "n+1 below setup").clear();

        let config = DatabaseConfig {
            n_plus_one_min_repeats: 4,
            n_plus_one_retention: 5,
            ..DatabaseConfig::default()
        };
        scope_http_sql_query_trace(
            Some(config),
            None,
            "GET".to_string(),
            "/users".to_string(),
            Some("req-below".to_string()),
            async {
                for _ in 0..3 {
                    record_sql_observation(
                        "SELECT * FROM users WHERE id = $1",
                        12,
                        None,
                        "query",
                        1,
                    );
                }
            },
        )
        .await;

        assert!(recent_n_plus_one_suspects().is_empty());
    }

    #[test]
    fn query_trace_does_not_record_outside_http_scope() {
        let _guard = sql_observability_test_lock().blocking_lock();
        lock_unpoisoned(n_plus_one_log(), "n+1 outside setup").clear();

        record_sql_observation(
            "SELECT * FROM scheduled_jobs WHERE id = $1",
            15,
            Some("scheduler"),
            "query",
            1,
        );

        assert!(recent_n_plus_one_suspects().is_empty());
    }

    #[tokio::test]
    async fn spawned_stream_cancels_worker_when_dropped() {
        struct DropFlag(Arc<AtomicBool>);

        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }

        let (_sender, receiver) = mpsc::channel::<Result<DbRecord>>(1);
        let (started_tx, started_rx) = oneshot::channel();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let aborted = Arc::new(AtomicBool::new(false));
        let aborted_flag = aborted.clone();
        let handle = tokio::spawn(async move {
            let _drop_flag = DropFlag(aborted_flag);
            let _ = started_tx.send(());
            let _ = cancel_rx.await;
        });
        started_rx.await.unwrap();

        let stream = receiver_stream(receiver, cancel_tx, handle);
        drop(stream);

        for _ in 0..20 {
            if aborted.load(Ordering::SeqCst) {
                return;
            }
            tokio::task::yield_now().await;
        }

        assert!(aborted.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn stream_worker_panic_becomes_stream_error() {
        let (sender, receiver) = mpsc::channel::<Result<DbRecord>>(1);
        let (cancel_tx, _cancel_rx) = oneshot::channel();
        let handle = spawn_stream_task(sender, async {
            panic!("stream boom");
            #[allow(unreachable_code)]
            Ok(())
        });

        let mut stream = receiver_stream(receiver, cancel_tx, handle);
        let error = stream
            .next()
            .await
            .expect("stream should yield panic error")
            .unwrap_err();

        assert_eq!(error.to_string(), "database stream panicked: stream boom");
    }
}
