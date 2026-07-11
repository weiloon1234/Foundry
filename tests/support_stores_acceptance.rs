use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use foundry::countries::seed_countries;
use foundry::prelude::*;
use foundry::settings::{NewSetting, Setting};
use tempfile::TempDir;
use tokio::sync::{Mutex, MutexGuard};

const METADATA_DOCUMENTS_TABLE: &str = "foundry_metadata_documents";

#[derive(Debug, foundry::Model)]
#[foundry(table = METADATA_DOCUMENTS_TABLE)]
struct MetadataDocument {
    id: ModelId<Self>,
    name: String,
    created_at: DateTime,
    updated_at: DateTime,
}

impl HasMetadata for MetadataDocument {
    fn metadatable_type() -> &'static str {
        METADATA_DOCUMENTS_TABLE
    }

    fn metadatable_id(&self) -> String {
        self.id.to_string()
    }
}

fn postgres_url() -> Option<String> {
    std::env::var("FOUNDRY_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

async fn support_stores_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().await
}

fn next_redis_namespace() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    format!(
        "support-stores-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

struct SupportStoresRuntime {
    _dir: TempDir,
    app: AppContext,
    database: std::sync::Arc<DatabaseManager>,
}

impl SupportStoresRuntime {
    async fn new() -> Option<Self> {
        let url = postgres_url()?;
        let redis_namespace = next_redis_namespace();
        let dir = tempfile::tempdir().ok()?;
        fs::write(
            dir.path().join("00-runtime.toml"),
            format!(
                r#"
                [database]
                url = "{url}"

                [redis]
                namespace = "{redis_namespace}"
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

        reset_settings(database.as_ref()).await;
        reset_notifications(database.as_ref()).await;
        reset_countries(database.as_ref()).await;
        reset_model_translations(database.as_ref()).await;
        reset_metadata(database.as_ref()).await;

        Some(Self {
            _dir: dir,
            app,
            database,
        })
    }

    async fn cleanup(&self) {
        self.app.shutdown().await.unwrap();
        let _ = self
            .database
            .raw_execute("DROP TABLE IF EXISTS settings", &[])
            .await;
        let _ = self
            .database
            .raw_execute("DROP TABLE IF EXISTS notifications", &[])
            .await;
        let _ = self
            .database
            .raw_execute("DROP TABLE IF EXISTS countries", &[])
            .await;
        let _ = self
            .database
            .raw_execute("DROP TABLE IF EXISTS model_translations", &[])
            .await;
        let _ = self
            .database
            .raw_execute("DROP TABLE IF EXISTS metadata", &[])
            .await;
        let _ = self
            .database
            .raw_execute(
                &format!("DROP TABLE IF EXISTS {METADATA_DOCUMENTS_TABLE}"),
                &[],
            )
            .await;
    }
}

async fn reset_metadata(database: &DatabaseManager) {
    database
        .raw_execute("DROP TABLE IF EXISTS metadata", &[])
        .await
        .unwrap();
    database
        .raw_execute(
            &format!("DROP TABLE IF EXISTS {METADATA_DOCUMENTS_TABLE}"),
            &[],
        )
        .await
        .unwrap();
    database
        .raw_execute(
            &format!(
                "CREATE TABLE {METADATA_DOCUMENTS_TABLE} (
                    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                    name TEXT NOT NULL,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                )"
            ),
            &[],
        )
        .await
        .unwrap();
    database
        .raw_execute(
            "CREATE TABLE metadata (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                metadatable_type TEXT NOT NULL,
                metadatable_id UUID NOT NULL,
                key TEXT NOT NULL,
                value JSONB,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ
            )",
            &[],
        )
        .await
        .unwrap();
    database
        .raw_execute(
            "CREATE UNIQUE INDEX idx_metadata_unique ON metadata (metadatable_type, metadatable_id, key)",
            &[],
        )
        .await
        .unwrap();
}

async fn reset_settings(database: &DatabaseManager) {
    database
        .raw_execute("DROP TABLE IF EXISTS settings", &[])
        .await
        .unwrap();
    database
        .raw_execute(
            r#"
            CREATE TABLE settings (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                key TEXT NOT NULL,
                value JSONB,
                setting_type TEXT NOT NULL DEFAULT 'text',
                parameters JSONB NOT NULL DEFAULT '{}',
                group_name TEXT NOT NULL DEFAULT 'general',
                label TEXT NOT NULL DEFAULT '',
                description TEXT,
                sort_order INT NOT NULL DEFAULT 0,
                is_public BOOLEAN NOT NULL DEFAULT false,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ
            )
            "#,
            &[],
        )
        .await
        .unwrap();
    database
        .raw_execute(
            "CREATE UNIQUE INDEX idx_settings_key ON settings (key)",
            &[],
        )
        .await
        .unwrap();
}

async fn reset_notifications(database: &DatabaseManager) {
    database
        .raw_execute("DROP TABLE IF EXISTS notifications", &[])
        .await
        .unwrap();
    database
        .raw_execute(
            r#"
            CREATE TABLE notifications (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                notifiable_type TEXT NOT NULL DEFAULT 'default',
                notifiable_id TEXT NOT NULL,
                type TEXT NOT NULL,
                data JSONB NOT NULL DEFAULT '{}',
                read_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
            &[],
        )
        .await
        .unwrap();
    database
        .raw_execute(
            "CREATE INDEX idx_notifications_notifiable ON notifications (notifiable_type, notifiable_id, created_at DESC, id DESC)",
            &[],
        )
        .await
        .unwrap();
    database
        .raw_execute(
            "CREATE INDEX idx_notifications_unread ON notifications (notifiable_type, notifiable_id, created_at DESC, id DESC) WHERE read_at IS NULL",
            &[],
        )
        .await
        .unwrap();
}

async fn reset_countries(database: &DatabaseManager) {
    database
        .raw_execute("DROP TABLE IF EXISTS countries", &[])
        .await
        .unwrap();
    database
        .raw_execute(
            r#"
            CREATE TABLE countries (
                iso2 CHAR(2) PRIMARY KEY,
                iso3 CHAR(3) NOT NULL,
                iso_numeric TEXT,
                name TEXT NOT NULL,
                official_name TEXT,
                capital TEXT,
                region TEXT,
                subregion TEXT,
                currencies JSONB NOT NULL DEFAULT '[]',
                primary_currency_code TEXT,
                calling_code TEXT,
                calling_root TEXT,
                calling_suffixes JSONB NOT NULL DEFAULT '[]',
                tlds JSONB NOT NULL DEFAULT '[]',
                timezones JSONB NOT NULL DEFAULT '[]',
                latitude DOUBLE PRECISION,
                longitude DOUBLE PRECISION,
                independent BOOLEAN,
                un_member BOOLEAN,
                flag_emoji TEXT,
                conversion_rate DOUBLE PRECISION,
                is_default BOOLEAN NOT NULL DEFAULT false,
                status TEXT NOT NULL DEFAULT 'disabled',
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ
            )
            "#,
            &[],
        )
        .await
        .unwrap();
}

async fn reset_model_translations(database: &DatabaseManager) {
    database
        .raw_execute("DROP TABLE IF EXISTS model_translations", &[])
        .await
        .unwrap();
    database
        .raw_execute(
            r#"
            CREATE TABLE model_translations (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                translatable_type TEXT NOT NULL,
                translatable_id TEXT NOT NULL,
                locale TEXT NOT NULL,
                field TEXT NOT NULL,
                value TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ,
                UNIQUE (translatable_type, translatable_id, locale, field)
            )
            "#,
            &[],
        )
        .await
        .unwrap();
}

struct ManualKeyTranslatable {
    id: String,
}

impl HasTranslations for ManualKeyTranslatable {
    fn translatable_type() -> &'static str {
        "manual_key_products"
    }

    fn translatable_id(&self) -> String {
        self.id.clone()
    }
}

#[tokio::test]
async fn translations_support_manual_text_and_integer_shaped_model_keys() {
    let _guard = support_stores_lock().await;
    let Some(runtime) = SupportStoresRuntime::new().await else {
        return;
    };

    for id in ["sku:desk/42", "9001"] {
        let model = ManualKeyTranslatable { id: id.to_string() };
        model
            .set_translation(&runtime.app, "en", "name", "Desk")
            .await
            .unwrap();
        model
            .set_translation(&runtime.app, "ms", "name", "Meja")
            .await
            .unwrap();

        assert_eq!(
            model.translation(&runtime.app, "ms", "name").await.unwrap(),
            Some("Meja".to_string())
        );
        let rows = model.all_translations(&runtime.app).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.translatable_id == id));

        assert_eq!(
            model.delete_all_translations(&runtime.app).await.unwrap(),
            2
        );
    }

    runtime.cleanup().await;
}

#[tokio::test]
async fn metadata_eager_loading_and_orphan_maintenance_are_first_class() {
    let _guard = support_stores_lock().await;
    let Some(runtime) = SupportStoresRuntime::new().await else {
        return;
    };

    let first = MetadataDocument::create()
        .set(MetadataDocument::NAME, "First")
        .save(&runtime.app)
        .await
        .unwrap();
    let second = MetadataDocument::create()
        .set(MetadataDocument::NAME, "Second")
        .save(&runtime.app)
        .await
        .unwrap();
    first.set_meta(&runtime.app, "theme", "dark").await.unwrap();
    first
        .set_meta(&runtime.app, "layout", "wide")
        .await
        .unwrap();
    second
        .set_meta(&runtime.app, "theme", "light")
        .await
        .unwrap();

    runtime
        .app
        .with_model_batching(async {
            let documents = MetadataDocument::query()
                .with_meta("theme")
                .order_by(MetadataDocument::NAME.asc())
                .get(&runtime.app)
                .await
                .unwrap();
            let first_theme: String = documents[0]
                .get_meta(&runtime.app, "theme")
                .await
                .unwrap()
                .unwrap();
            let second_theme: String = documents[1]
                .get_meta(&runtime.app, "theme")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(first_theme, "dark");
            assert_eq!(second_theme, "light");
        })
        .await;

    runtime
        .app
        .with_model_batching(async {
            let documents = MetadataDocument::query()
                .with_metadata()
                .order_by(MetadataDocument::NAME.asc())
                .get(&runtime.app)
                .await
                .unwrap();
            assert_eq!(documents[0].all_meta(&runtime.app).await.unwrap().len(), 2);
            assert_eq!(
                documents[0]
                    .get_meta::<String>(&runtime.app, "layout")
                    .await
                    .unwrap()
                    .as_deref(),
                Some("wide")
            );
        })
        .await;

    runtime
        .database
        .raw_execute(
            "UPDATE metadata SET value = NULL WHERE metadatable_type = $1 AND metadatable_id = $2 AND key = 'theme'",
            &[
                DbValue::Text(MetadataDocument::metadatable_type().to_string()),
                DbValue::Uuid(second.id.into_uuid()),
            ],
        )
        .await
        .unwrap();
    assert!(second.has_meta(&runtime.app, "theme").await.unwrap());

    runtime
        .database
        .raw_execute(
            &format!("DELETE FROM {METADATA_DOCUMENTS_TABLE} WHERE id = $1"),
            &[DbValue::Uuid(second.id.into_uuid())],
        )
        .await
        .unwrap();
    let owner = MetadataOwner::for_model::<MetadataDocument>().unwrap();
    assert_eq!(
        audit_metadata_orphans(runtime.database.as_ref(), &owner)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        prune_metadata_orphans(runtime.database.as_ref(), &owner)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        audit_metadata_orphans(runtime.database.as_ref(), &owner)
            .await
            .unwrap(),
        0
    );
    assert_eq!(first.delete_all_meta(&runtime.app).await.unwrap(), 2);
    assert!(first.all_meta(&runtime.app).await.unwrap().is_empty());

    runtime.cleanup().await;
}

#[tokio::test]
async fn settings_crud_lists_and_prefix_filters_use_typed_queries() {
    let _guard = support_stores_lock().await;
    let Some(runtime) = SupportStoresRuntime::new().await else {
        return;
    };

    Setting::create(
        &runtime.app,
        NewSetting::new("feature.alpha", "Alpha")
            .value(serde_json::json!(true))
            .group("features")
            .sort_order(2)
            .is_public(true),
    )
    .await
    .unwrap();
    Setting::create(
        &runtime.app,
        NewSetting::new("feature.percent%literal", "Percent Literal").group("features"),
    )
    .await
    .unwrap();
    Setting::create(
        &runtime.app,
        NewSetting::new("feature.percentXliteral", "Percent Wildcard").group("features"),
    )
    .await
    .unwrap();
    Setting::upsert(&runtime.app, "feature.beta", serde_json::json!("beta"))
        .await
        .unwrap();
    Setting::set(&runtime.app, "feature.alpha", serde_json::json!(false))
        .await
        .unwrap();
    let missing_error = Setting::set(&runtime.app, "feature.missing", serde_json::json!(true))
        .await
        .unwrap_err();
    assert!(missing_error
        .to_string()
        .contains("setting `feature.missing` does not exist"));
    assert!(!Setting::exists(&runtime.app, "feature.missing")
        .await
        .unwrap());

    assert_eq!(
        Setting::get(&runtime.app, "feature.alpha").await.unwrap(),
        Some(serde_json::json!(false))
    );
    assert!(Setting::exists(&runtime.app, "feature.beta").await.unwrap());
    assert_eq!(
        Setting::find(&runtime.app, "feature.alpha")
            .await
            .unwrap()
            .unwrap()
            .group_name,
        "features"
    );
    assert_eq!(
        Setting::groups(&runtime.app).await.unwrap(),
        vec!["features", "general"]
    );
    assert_eq!(Setting::public(&runtime.app).await.unwrap().len(), 1);
    assert_eq!(
        Setting::by_group(&runtime.app, "features")
            .await
            .unwrap()
            .len(),
        3
    );

    let literal_prefix = Setting::by_prefix(&runtime.app, "feature.percent%")
        .await
        .unwrap();
    assert_eq!(literal_prefix.len(), 1);
    assert_eq!(literal_prefix[0].key, "feature.percent%literal");

    assert!(Setting::remove(&runtime.app, "feature.beta").await.unwrap());
    assert!(!Setting::exists(&runtime.app, "feature.beta").await.unwrap());

    runtime
        .database
        .raw_execute(
            "UPDATE settings SET setting_type = 'unsupported-widget' WHERE key = 'feature.alpha'",
            &[],
        )
        .await
        .unwrap();
    let drift_error = Setting::find(&runtime.app, "feature.alpha")
        .await
        .unwrap_err();
    assert!(drift_error
        .to_string()
        .contains("setting `feature.alpha` has unknown setting type `unsupported-widget`"));

    runtime.cleanup().await;
}

struct TestNotifiable;

impl Notifiable for TestNotifiable {
    fn notification_id(&self) -> String {
        "user-42".to_string()
    }
}

struct TypedTestNotifiable;

impl Notifiable for TypedTestNotifiable {
    fn notifiable_type(&self) -> &str {
        "admin"
    }

    fn notification_id(&self) -> String {
        "user-42".to_string()
    }
}

struct DatabaseOnlyNotification;

impl Notification for DatabaseOnlyNotification {
    fn notification_type(&self) -> &str {
        "database_only"
    }

    fn via(&self) -> Vec<NotificationChannelId> {
        vec![NOTIFY_DATABASE]
    }

    fn to_database(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "message": "stored" }))
    }
}

struct DatabaseAndBroadcastNotification;

impl Notification for DatabaseAndBroadcastNotification {
    fn notification_type(&self) -> &str {
        "database_and_broadcast"
    }

    fn via(&self) -> Vec<NotificationChannelId> {
        vec![NOTIFY_DATABASE, NOTIFY_BROADCAST]
    }

    fn to_database(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "message": "stored after commit" }))
    }

    fn to_broadcast(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "message": "broadcast after commit" }))
    }
}

#[tokio::test]
async fn database_notifications_are_stored_with_typed_queries() {
    let _guard = support_stores_lock().await;
    let Some(runtime) = SupportStoresRuntime::new().await else {
        return;
    };

    runtime
        .app
        .notify(&TestNotifiable, &DatabaseOnlyNotification)
        .await
        .unwrap();

    let rows = runtime
        .database
        .raw_query(
            "SELECT notifiable_type, notifiable_id, type, data FROM notifications ORDER BY created_at",
            &[],
        )
        .await
        .unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].try_text("notifiable_type").unwrap(),
        DEFAULT_NOTIFIABLE_TYPE
    );
    assert_eq!(rows[0].try_text("notifiable_id").unwrap(), "user-42");
    assert_eq!(rows[0].try_text("type").unwrap(), "database_only");
    assert_eq!(
        rows[0].decode::<serde_json::Value>("data").unwrap(),
        serde_json::json!({ "message": "stored" })
    );

    runtime
        .app
        .notify(&TypedTestNotifiable, &DatabaseOnlyNotification)
        .await
        .unwrap();
    let default_repository =
        DatabaseNotificationRepository::for_notifiable(&TestNotifiable).unwrap();
    let typed_repository =
        DatabaseNotificationRepository::for_notifiable(&TypedTestNotifiable).unwrap();
    assert_eq!(
        default_repository.list(&runtime.app).await.unwrap().len(),
        1
    );
    assert_eq!(typed_repository.list(&runtime.app).await.unwrap().len(), 1);

    runtime.cleanup().await;
}

#[tokio::test]
async fn queued_notification_channels_are_independent_after_commit_callbacks() {
    let _guard = support_stores_lock().await;
    let Some(runtime) = SupportStoresRuntime::new().await else {
        return;
    };
    let worker = Worker::from_app(runtime.app.clone()).unwrap();

    let transaction = runtime.app.begin_transaction().await.unwrap();
    transaction
        .notify_after_commit(&TestNotifiable, &DatabaseAndBroadcastNotification)
        .unwrap();
    transaction.rollback().await.unwrap();
    assert!(!worker.run_once().await.unwrap());

    let transaction = runtime.app.begin_transaction().await.unwrap();
    transaction
        .notify_after_commit(&TestNotifiable, &DatabaseAndBroadcastNotification)
        .unwrap();
    transaction.commit().await.unwrap();

    assert!(worker.run_once().await.unwrap());
    assert!(worker.run_once().await.unwrap());
    assert!(!worker.run_once().await.unwrap());
    let repository = DatabaseNotificationRepository::new(
        DEFAULT_NOTIFIABLE_TYPE,
        TestNotifiable.notification_id(),
    )
    .unwrap();
    let stored = repository.list(&runtime.app).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].notification_type, "database_and_broadcast");

    runtime.cleanup().await;
}

#[tokio::test]
async fn database_notification_repository_scopes_orders_and_mutates_with_transactions() {
    let _guard = support_stores_lock().await;
    let Some(runtime) = SupportStoresRuntime::new().await else {
        return;
    };

    runtime
        .database
        .raw_execute(
            r#"
            INSERT INTO notifications
                (id, notifiable_type, notifiable_id, type, data, read_at, created_at)
            VALUES
                ('00000000-0000-0000-0000-000000000001', 'user', 'shared-1', 'first', '{"position":1}', NULL, '2026-07-10T12:00:00Z'),
                ('00000000-0000-0000-0000-000000000002', 'user', 'shared-1', 'second', '{"position":2}', '2026-07-10T12:02:00Z', '2026-07-10T12:01:00Z'),
                ('00000000-0000-0000-0000-000000000003', 'user', 'shared-1', 'third', '{"position":3}', NULL, '2026-07-10T12:01:00Z'),
                ('00000000-0000-0000-0000-000000000004', 'team', 'shared-1', 'team-only', '{}', NULL, '2026-07-10T12:03:00Z'),
                ('00000000-0000-0000-0000-000000000005', 'user', 'other-1', 'other-user', '{}', NULL, '2026-07-10T12:04:00Z')
            "#,
            &[],
        )
        .await
        .unwrap();

    let repository = DatabaseNotificationRepository::new("user", "shared-1").unwrap();
    let team_repository = DatabaseNotificationRepository::new("team", "shared-1").unwrap();
    let other_repository = DatabaseNotificationRepository::new("user", "other-1").unwrap();
    let id = |value: &str| ModelId::<DatabaseNotification>::parse_str(value).unwrap();
    let first_id = id("00000000-0000-0000-0000-000000000001");
    let third_id = id("00000000-0000-0000-0000-000000000003");
    let team_id = id("00000000-0000-0000-0000-000000000004");

    let notifications = repository.list(&runtime.app).await.unwrap();
    assert_eq!(
        notifications
            .iter()
            .map(|notification| notification.notification_type.as_str())
            .collect::<Vec<_>>(),
        ["third", "second", "first"]
    );
    let page = repository
        .paginate(&runtime.app, Pagination::new(1, 2))
        .await
        .unwrap();
    assert_eq!(page.total, 3);
    assert_eq!(page.data.len(), 2);
    assert_eq!(page.data[0].id, third_id);
    assert_eq!(page.data[1].notification_type, "second");

    assert_eq!(repository.unread_count(&runtime.app).await.unwrap(), 2);
    assert_eq!(repository.unread(&runtime.app).await.unwrap().len(), 2);
    assert_eq!(repository.read(&runtime.app).await.unwrap().len(), 1);
    assert_eq!(team_repository.unread_count(&runtime.app).await.unwrap(), 1);
    assert_eq!(
        other_repository.unread_count(&runtime.app).await.unwrap(),
        1
    );
    assert!(!repository.delete(&runtime.app, team_id).await.unwrap());

    let transaction = runtime.app.begin_transaction().await.unwrap();
    assert!(repository
        .mark_read_with(&transaction, third_id)
        .await
        .unwrap());
    assert_eq!(repository.unread_count_with(&transaction).await.unwrap(), 1);
    transaction.rollback().await.unwrap();
    assert_eq!(repository.unread_count(&runtime.app).await.unwrap(), 2);

    assert!(repository.mark_read(&runtime.app, third_id).await.unwrap());
    assert!(!repository.mark_read(&runtime.app, third_id).await.unwrap());
    assert_eq!(repository.mark_all_read(&runtime.app).await.unwrap(), 1);
    assert_eq!(repository.unread_count(&runtime.app).await.unwrap(), 0);
    assert_eq!(team_repository.unread_count(&runtime.app).await.unwrap(), 1);
    assert_eq!(
        other_repository.unread_count(&runtime.app).await.unwrap(),
        1
    );

    assert!(repository.delete(&runtime.app, first_id).await.unwrap());
    assert!(!repository.delete(&runtime.app, first_id).await.unwrap());
    assert_eq!(repository.list(&runtime.app).await.unwrap().len(), 2);

    runtime.cleanup().await;
}

#[tokio::test]
async fn countries_seed_and_read_helpers_use_typed_queries() {
    let _guard = support_stores_lock().await;
    let Some(runtime) = SupportStoresRuntime::new().await else {
        return;
    };

    let seeded = seed_countries(&runtime.app).await.unwrap();
    assert_eq!(seeded, 250);

    let malaysia = Country::find(&runtime.app, "my").await.unwrap().unwrap();
    assert_eq!(malaysia.iso2, "MY");
    assert_eq!(malaysia.iso3, "MYS");
    assert_eq!(malaysia.currencies[0].code, "MYR");
    assert_eq!(malaysia.calling_code.as_deref(), Some("+60"));
    assert!(malaysia.calling_suffixes.iter().any(|suffix| suffix == "0"));
    assert!(malaysia.tlds.iter().any(|tld| tld == ".my"));
    assert_eq!(
        malaysia.timezones,
        vec!["Asia/Kuala_Lumpur", "Asia/Kuching"]
    );
    let malaysia_json = serde_json::to_value(&malaysia).unwrap();
    assert!(malaysia_json["currencies"].is_array());
    assert!(malaysia_json["calling_suffixes"].is_array());
    assert!(malaysia_json["tlds"].is_array());
    assert!(malaysia_json["timezones"].is_array());
    assert!(Country::exists(&runtime.app, "my").await.unwrap());
    assert_eq!(Country::all(&runtime.app).await.unwrap().len(), 250);
    assert_eq!(Country::enabled(&runtime.app).await.unwrap().len(), 0);

    runtime
        .database
        .raw_execute(
            "UPDATE countries SET status = 'enabled' WHERE iso2 = 'MY'",
            &[],
        )
        .await
        .unwrap();
    assert!(Country::enabled(&runtime.app)
        .await
        .unwrap()
        .iter()
        .any(|country| country.iso2 == "MY"));

    seed_countries(&runtime.app).await.unwrap();
    assert!(Country::find(&runtime.app, "MY")
        .await
        .unwrap()
        .unwrap()
        .status
        .is_enabled());

    runtime
        .database
        .raw_execute(
            "UPDATE countries SET status = 'archived' WHERE iso2 = 'MY'",
            &[],
        )
        .await
        .unwrap();
    let status_error = Country::find(&runtime.app, "MY").await.unwrap_err();
    assert!(status_error
        .to_string()
        .contains("country `MY` has unknown status `archived`"));

    runtime
        .database
        .raw_execute(
            "UPDATE countries SET status = 'enabled', currencies = '{}'::jsonb WHERE iso2 = 'MY'",
            &[],
        )
        .await
        .unwrap();
    let collection_error = Country::find(&runtime.app, "MY").await.unwrap_err();
    assert!(collection_error.to_string().contains("country `MY`"));
    assert!(collection_error.to_string().contains("field `currencies`"));

    runtime.cleanup().await;
}
