use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::OnceLock;

use async_trait::async_trait;
use foundry::prelude::*;
use tempfile::TempDir;
use tokio::sync::{Mutex, MutexGuard};

const AUDIT_LOGS_TABLE: &str = "audit_logs";
const AUDIT_ENTRIES_TABLE: &str = "foundry_test_audit_entries";
const NO_AUDIT_ENTRIES_TABLE: &str = "foundry_test_no_audit_entries";
const REQUEST_ID_HEADER: &str = "x-request-id";

fn postgres_url() -> Option<String> {
    std::env::var("FOUNDRY_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

async fn audit_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().await
}

struct AuditRuntime {
    _dir: TempDir,
    app: AppContext,
    database: Arc<DatabaseManager>,
}

impl AuditRuntime {
    async fn new() -> Option<Self> {
        let url = postgres_url()?;
        let dir = tempfile::tempdir().ok()?;
        fs::write(
            dir.path().join("00-runtime.toml"),
            format!(
                r#"
                [database]
                url = "{url}"
                "#
            ),
        )
        .ok()?;

        let kernel = App::builder()
            .load_config_dir(dir.path())
            .build_cli_kernel()
            .await
            .ok()?;
        let app = kernel.app().clone();
        let database = app.database().ok()?;

        Some(Self {
            _dir: dir,
            app,
            database,
        })
    }

    fn config_dir(&self) -> &Path {
        self._dir.path()
    }
}

async fn build_test_app(config_dir: &Path) -> TestApp {
    TestApp::builder()
        .load_config_dir(config_dir)
        .register_provider(AuditAuthProvider)
        .register_middleware(TrustedProxy::new().trust_all().build())
        .register_routes(audit_routes)
        .build()
        .await
        .unwrap()
}

#[derive(Clone)]
struct AuditAuthProvider;

#[async_trait]
impl ServiceProvider for AuditAuthProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.register_guard(
            GuardId::new("admin"),
            StaticBearerAuthenticator::new()
                .token("admin-token", Actor::new("admin-1", GuardId::new("admin"))),
        )?;
        Ok(())
    }
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = AUDIT_ENTRIES_TABLE, primary_key_strategy = "manual")]
struct AuditEntry {
    id: i64,
    title: String,
    #[foundry(audit_exclude)]
    secret: String,
    created_at: DateTime,
    updated_at: DateTime,
    deleted_at: Option<DateTime>,
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = NO_AUDIT_ENTRIES_TABLE, primary_key_strategy = "manual", audit = false)]
struct NoAuditEntry {
    id: i64,
    title: String,
    created_at: DateTime,
    updated_at: DateTime,
}

fn audit_routes(registrar: &mut HttpRegistrar) -> Result<()> {
    registrar.scope("/admin", |admin| {
        admin
            .name_prefix("admin")
            .guard(GuardId::new("admin"))
            .audit_area("admin");

        admin.post("/audit-entry", "audit_entry", create_audit_entry, |_| {});
        admin.post(
            "/audit-entry/commit",
            "audit_entry_commit",
            create_audit_entry_in_transaction_and_commit,
            |_| {},
        );
        admin.post(
            "/audit-entry/rollback",
            "audit_entry_rollback",
            create_audit_entry_in_transaction_and_rollback,
            |_| {},
        );
        admin.post(
            "/audit-entry/lifecycle",
            "audit_entry_lifecycle",
            create_update_and_delete_audit_entry,
            |_| {},
        );
        admin.post(
            "/no-audit-entry",
            "no_audit_entry",
            create_no_audit_entry,
            |_| {},
        );

        admin.scope("/sensitive", |sensitive| {
            sensitive.name_prefix("sensitive").audit_disabled();
            sensitive.post(
                "/audit-entry",
                "audit_entry",
                create_disabled_audit_entry,
                |_| {},
            );
            Ok(())
        })?;

        admin.scope("/support", |support| {
            support.name_prefix("support").audit_area("support");
            support.post(
                "/audit-entry",
                "audit_entry",
                create_support_area_audit_entry,
                |_| {},
            );
            Ok(())
        })?;

        Ok(())
    })?;

    registrar.scope("/plain", |plain| {
        plain.name_prefix("plain").guard(GuardId::new("admin"));
        plain.post(
            "/audit-entry",
            "audit_entry",
            create_plain_audit_entry,
            |_| {},
        );
        Ok(())
    })?;

    Ok(())
}

async fn create_audit_entry(State(app): State<AppContext>, _actor: CurrentActor) -> StatusCode {
    AuditEntry::create()
        .set(AuditEntry::ID, 101_i64)
        .set(AuditEntry::TITLE, "Created over HTTP")
        .set(AuditEntry::SECRET, "never-log-this")
        .save(&app)
        .await
        .unwrap();

    StatusCode::CREATED
}

async fn create_plain_audit_entry(
    State(app): State<AppContext>,
    _actor: CurrentActor,
) -> StatusCode {
    AuditEntry::create()
        .set(AuditEntry::ID, 104_i64)
        .set(AuditEntry::TITLE, "Created without area")
        .set(AuditEntry::SECRET, "plain-secret")
        .save(&app)
        .await
        .unwrap();

    StatusCode::CREATED
}

async fn create_disabled_audit_entry(
    State(app): State<AppContext>,
    _actor: CurrentActor,
) -> StatusCode {
    AuditEntry::create()
        .set(AuditEntry::ID, 102_i64)
        .set(AuditEntry::TITLE, "Created with audit disabled")
        .set(AuditEntry::SECRET, "disabled-secret")
        .save(&app)
        .await
        .unwrap();

    StatusCode::CREATED
}

async fn create_support_area_audit_entry(
    State(app): State<AppContext>,
    _actor: CurrentActor,
) -> StatusCode {
    AuditEntry::create()
        .set(AuditEntry::ID, 103_i64)
        .set(AuditEntry::TITLE, "Created in support area")
        .set(AuditEntry::SECRET, "support-secret")
        .save(&app)
        .await
        .unwrap();

    StatusCode::CREATED
}

async fn create_no_audit_entry(State(app): State<AppContext>, _actor: CurrentActor) -> StatusCode {
    NoAuditEntry::create()
        .set(NoAuditEntry::ID, 1_i64)
        .set(NoAuditEntry::TITLE, "Sensitive model")
        .save(&app)
        .await
        .unwrap();

    StatusCode::CREATED
}

async fn create_audit_entry_in_transaction_and_commit(
    State(app): State<AppContext>,
    _actor: CurrentActor,
) -> Result<StatusCode> {
    let tx = app.begin_transaction().await?;
    AuditEntry::create()
        .set(AuditEntry::ID, 301_i64)
        .set(AuditEntry::TITLE, "Committed")
        .set(AuditEntry::SECRET, "commit-secret")
        .save(&tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::CREATED)
}

async fn create_audit_entry_in_transaction_and_rollback(
    State(app): State<AppContext>,
    _actor: CurrentActor,
) -> Result<StatusCode> {
    let tx = app.begin_transaction().await?;
    AuditEntry::create()
        .set(AuditEntry::ID, 302_i64)
        .set(AuditEntry::TITLE, "Rolled back")
        .set(AuditEntry::SECRET, "rollback-secret")
        .save(&tx)
        .await?;
    tx.rollback().await?;
    Err(Error::http(500, "rolled back"))
}

async fn create_update_and_delete_audit_entry(
    State(app): State<AppContext>,
    _actor: CurrentActor,
) -> StatusCode {
    AuditEntry::create()
        .set(AuditEntry::ID, 10_i64)
        .set(AuditEntry::TITLE, "Draft")
        .set(AuditEntry::SECRET, "hidden-1")
        .save(&app)
        .await
        .unwrap();

    AuditEntry::update()
        .where_(AuditEntry::ID.eq(10_i64))
        .set(AuditEntry::TITLE, "Published")
        .set(AuditEntry::SECRET, "hidden-2")
        .save(&app)
        .await
        .unwrap();

    AuditEntry::delete()
        .where_(AuditEntry::ID.eq(10_i64))
        .execute(&app)
        .await
        .unwrap();

    AuditEntry::restore()
        .where_(AuditEntry::ID.eq(10_i64))
        .save(&app)
        .await
        .unwrap();

    AuditEntry::force_delete()
        .where_(AuditEntry::ID.eq(10_i64))
        .execute(&app)
        .await
        .unwrap();

    StatusCode::NO_CONTENT
}

async fn execute_batch(database: &DatabaseManager, statements: &[&str]) {
    for statement in statements {
        database.raw_execute(statement, &[]).await.unwrap();
    }
}

async fn reset_schema(database: &DatabaseManager) {
    execute_batch(
        database,
        &[
            &format!("DROP TABLE IF EXISTS {NO_AUDIT_ENTRIES_TABLE}"),
            &format!("DROP TABLE IF EXISTS {AUDIT_ENTRIES_TABLE}"),
            &format!("DROP TABLE IF EXISTS {AUDIT_LOGS_TABLE}"),
            &format!(
                "CREATE TABLE {AUDIT_LOGS_TABLE} (
                    id UUID PRIMARY KEY DEFAULT uuidv7(),
                    event_type TEXT NOT NULL,
                    subject_model TEXT NOT NULL,
                    subject_table TEXT NOT NULL,
                    subject_id TEXT NOT NULL,
                    area TEXT,
                    actor_guard TEXT,
                    actor_id TEXT,
                    request_id TEXT,
                    ip TEXT,
                    user_agent TEXT,
                    before_data JSONB,
                    after_data JSONB,
                    changes JSONB,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )"
            ),
            &format!(
                "CREATE TABLE {AUDIT_ENTRIES_TABLE} (
                    id BIGINT PRIMARY KEY,
                    title TEXT NOT NULL,
                    secret TEXT NOT NULL,
                    created_at TIMESTAMPTZ NOT NULL,
                    updated_at TIMESTAMPTZ NOT NULL,
                    deleted_at TIMESTAMPTZ NULL
                )"
            ),
            &format!(
                "CREATE TABLE {NO_AUDIT_ENTRIES_TABLE} (
                    id BIGINT PRIMARY KEY,
                    title TEXT NOT NULL,
                    created_at TIMESTAMPTZ NOT NULL,
                    updated_at TIMESTAMPTZ NOT NULL
                )"
            ),
        ],
    )
    .await;
}

async fn audit_logs_for_subject<E>(
    executor: &E,
    subject_table: &str,
    subject_id: impl ToString,
) -> Vec<AuditLog>
where
    E: QueryExecutor,
{
    AuditLog::query()
        .where_(AuditLog::SUBJECT_TABLE.eq(subject_table))
        .where_(AuditLog::SUBJECT_ID.eq(subject_id.to_string()))
        .order_by(AuditLog::CREATED_AT.asc())
        .order_by(AuditLog::ID.asc())
        .get(executor)
        .await
        .unwrap()
        .into_vec()
}

async fn latest_audit_log<E>(
    executor: &E,
    subject_table: &str,
    subject_id: impl ToString,
) -> AuditLog
where
    E: QueryExecutor,
{
    audit_logs_for_subject(executor, subject_table, subject_id)
        .await
        .into_iter()
        .last()
        .unwrap()
}

fn json_object(value: &serde_json::Value) -> &serde_json::Map<String, serde_json::Value> {
    value.as_object().unwrap()
}

#[tokio::test]
async fn audit_rows_commit_and_rollback_with_parent_transaction() {
    let _guard = audit_lock().await;
    let Some(runtime) = AuditRuntime::new().await else {
        return;
    };

    let app = build_test_app(runtime.config_dir()).await;
    reset_schema(app.app().database().unwrap().as_ref()).await;

    let rollback = app
        .client()
        .post("/admin/audit-entry/rollback")
        .bearer_auth("admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(rollback.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(audit_logs_for_subject(app.app(), AUDIT_ENTRIES_TABLE, 302)
        .await
        .is_empty());
    assert!(AuditEntry::query()
        .where_(AuditEntry::ID.eq(302_i64))
        .first(app.app())
        .await
        .unwrap()
        .is_none());

    let commit = app
        .client()
        .post("/admin/audit-entry/commit")
        .bearer_auth("admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(commit.status(), StatusCode::CREATED);

    let committed = audit_logs_for_subject(app.app(), AUDIT_ENTRIES_TABLE, 301).await;
    assert_eq!(committed.len(), 1);
    assert_eq!(committed[0].event_type, "created");
    assert_eq!(committed[0].area.as_deref(), Some("admin"));
    assert!(AuditEntry::query()
        .where_(AuditEntry::ID.eq(301_i64))
        .first(app.app())
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn http_writes_capture_actor_request_origin_and_area() {
    let _guard = audit_lock().await;
    let Some(runtime) = AuditRuntime::new().await else {
        return;
    };

    let app = build_test_app(runtime.config_dir()).await;
    reset_schema(app.app().database().unwrap().as_ref()).await;

    let response = app
        .client()
        .post("/admin/audit-entry")
        .bearer_auth("admin-token")
        .header(REQUEST_ID_HEADER, "req-audit-http")
        .header("x-forwarded-for", "203.0.113.5, 10.0.0.1")
        .header("user-agent", "FoundryAuditAcceptance/1.0")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let log = latest_audit_log(app.app(), AUDIT_ENTRIES_TABLE, 101).await;
    assert_eq!(log.event_type, "created");
    assert_eq!(log.subject_table, AUDIT_ENTRIES_TABLE);
    assert_eq!(log.area.as_deref(), Some("admin"));
    assert_eq!(log.actor_guard.as_deref(), Some("admin"));
    assert_eq!(log.actor_id.as_deref(), Some("admin-1"));
    assert_eq!(log.request_id.as_deref(), Some("req-audit-http"));
    assert_eq!(log.ip.as_deref(), Some("203.0.113.5"));
    assert_eq!(
        log.user_agent.as_deref(),
        Some("FoundryAuditAcceptance/1.0")
    );

    let after = log.after_data.unwrap();
    assert_eq!(after["title"], "Created over HTTP");
    assert!(json_object(&after).get("secret").is_none());
}

#[tokio::test]
async fn unmarked_and_disabled_routes_do_not_audit_but_explicit_areas_do() {
    let _guard = audit_lock().await;
    let Some(runtime) = AuditRuntime::new().await else {
        return;
    };

    let app = build_test_app(runtime.config_dir()).await;
    reset_schema(app.app().database().unwrap().as_ref()).await;

    let plain = app
        .client()
        .post("/plain/audit-entry")
        .bearer_auth("admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(plain.status(), StatusCode::CREATED);
    assert!(audit_logs_for_subject(app.app(), AUDIT_ENTRIES_TABLE, 104)
        .await
        .is_empty());

    let disabled = app
        .client()
        .post("/admin/sensitive/audit-entry")
        .bearer_auth("admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(disabled.status(), StatusCode::CREATED);
    assert!(audit_logs_for_subject(app.app(), AUDIT_ENTRIES_TABLE, 102)
        .await
        .is_empty());

    let support = app
        .client()
        .post("/admin/support/audit-entry")
        .bearer_auth("admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(support.status(), StatusCode::CREATED);
    let support_logs = audit_logs_for_subject(app.app(), AUDIT_ENTRIES_TABLE, 103).await;
    assert_eq!(support_logs.len(), 1);
    assert_eq!(support_logs[0].area.as_deref(), Some("support"));
}

#[tokio::test]
async fn direct_non_http_writes_do_not_audit_by_default() {
    let _guard = audit_lock().await;
    let Some(runtime) = AuditRuntime::new().await else {
        return;
    };

    reset_schema(runtime.database.as_ref()).await;

    AuditEntry::create()
        .set(AuditEntry::ID, 11_i64)
        .set(AuditEntry::TITLE, "Direct write")
        .set(AuditEntry::SECRET, "direct-secret")
        .save(&runtime.app)
        .await
        .unwrap();

    assert!(
        audit_logs_for_subject(&runtime.app, AUDIT_ENTRIES_TABLE, 11)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn admin_area_tracks_event_types_and_excludes_sensitive_fields() {
    let _guard = audit_lock().await;
    let Some(runtime) = AuditRuntime::new().await else {
        return;
    };

    let app = build_test_app(runtime.config_dir()).await;
    reset_schema(app.app().database().unwrap().as_ref()).await;

    let response = app
        .client()
        .post("/admin/audit-entry/lifecycle")
        .bearer_auth("admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let logs = audit_logs_for_subject(app.app(), AUDIT_ENTRIES_TABLE, 10).await;
    let event_types = logs
        .iter()
        .map(|log| log.event_type.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        event_types,
        vec!["created", "updated", "soft_deleted", "restored", "deleted"]
    );
    assert!(logs.iter().all(|log| log.area.as_deref() == Some("admin")));

    let created_after = logs[0].after_data.as_ref().unwrap();
    assert_eq!(created_after["title"], "Draft");
    assert!(json_object(created_after).get("secret").is_none());

    let updated_log = &logs[1];
    let updated_before = updated_log.before_data.as_ref().unwrap();
    let updated_after = updated_log.after_data.as_ref().unwrap();
    let updated_changes = updated_log.changes.as_ref().unwrap();
    assert_eq!(updated_before["title"], "Draft");
    assert_eq!(updated_after["title"], "Published");
    assert_eq!(updated_changes["title"]["before"], "Draft");
    assert_eq!(updated_changes["title"]["after"], "Published");
    assert!(json_object(updated_before).get("secret").is_none());
    assert!(json_object(updated_after).get("secret").is_none());
    assert!(json_object(updated_changes).get("secret").is_none());

    let soft_deleted_log = &logs[2];
    let soft_deleted_changes = soft_deleted_log.changes.as_ref().unwrap();
    assert_eq!(soft_deleted_log.event_type, "soft_deleted");
    assert!(json_object(soft_deleted_changes).contains_key("deleted_at"));

    let restored_log = &logs[3];
    let restored_changes = restored_log.changes.as_ref().unwrap();
    assert_eq!(restored_log.event_type, "restored");
    assert_eq!(
        restored_changes["deleted_at"]["after"],
        serde_json::Value::Null
    );

    let deleted_log = &logs[4];
    assert_eq!(deleted_log.event_type, "deleted");
    assert!(deleted_log.after_data.is_none());
    assert_eq!(
        deleted_log.before_data.as_ref().unwrap()["title"],
        "Published"
    );
}

#[tokio::test]
async fn model_level_opt_out_still_suppresses_audit_inside_active_area() {
    let _guard = audit_lock().await;
    let Some(runtime) = AuditRuntime::new().await else {
        return;
    };

    let app = build_test_app(runtime.config_dir()).await;
    reset_schema(app.app().database().unwrap().as_ref()).await;

    let response = app
        .client()
        .post("/admin/no-audit-entry")
        .bearer_auth("admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert!(audit_logs_for_subject(app.app(), NO_AUDIT_ENTRIES_TABLE, 1)
        .await
        .is_empty());
}

#[tokio::test]
async fn audit_log_model_does_not_recurse_and_null_area_rows_hydrate() {
    let _guard = audit_lock().await;
    let Some(runtime) = AuditRuntime::new().await else {
        return;
    };

    reset_schema(runtime.database.as_ref()).await;

    AuditLog::create()
        .set(AuditLog::EVENT_TYPE, "manual")
        .set(AuditLog::SUBJECT_MODEL, "audit_acceptance::Manual")
        .set(AuditLog::SUBJECT_TABLE, "manual_subjects")
        .set(AuditLog::SUBJECT_ID, "legacy-1")
        .save(&runtime.app)
        .await
        .unwrap();

    let log = latest_audit_log(&runtime.app, "manual_subjects", "legacy-1").await;
    assert_eq!(log.event_type, "manual");
    assert_eq!(log.subject_table, "manual_subjects");
    assert!(log.area.is_none());

    let logs = AuditLog::query()
        .order_by(AuditLog::CREATED_AT.asc())
        .order_by(AuditLog::ID.asc())
        .get(&runtime.app)
        .await
        .unwrap()
        .into_vec();

    assert_eq!(logs.len(), 1);
}
