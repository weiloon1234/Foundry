use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use async_trait::async_trait;
use foundry::config::DatabaseConfig;
use foundry::prelude::*;
use tempfile::TempDir;
use tokio::sync::{Mutex, MutexGuard};

const FIRST_GENERATED_MIGRATION_ID: &str = "000000000000_create_database_primitives";
const COUNTRIES_SEED_ID: &str = "000000000001_countries_seeder";
const GENERATED_MIGRATION_IDS: &[&str] = &[
    "000000000000_create_database_primitives",
    "000000000001_create_personal_access_tokens",
    "000000000002_create_password_reset_tokens",
    "000000000003_create_notifications",
    "000000000004_create_job_history",
    "000000000005_create_attachments",
    "000000000006_create_metadata",
    "000000000007_create_model_translations",
    "000000000008_create_countries",
    "000000000009_create_settings",
    "000000000010_create_audit_logs",
    "000000000011_create_auth_mfa_totp_factors",
    "000000000012_index_job_history_created_at",
    "000000000013_alter_model_translation_ids_to_text",
    "000000000014_add_notification_notifiable_type",
];
const GENERATED_TABLES: &[&str] = &[
    "personal_access_tokens",
    "password_reset_tokens",
    "notifications",
    "job_history",
    "attachments",
    "metadata",
    "model_translations",
    "countries",
    "settings",
    "audit_logs",
    "auth_mfa_totp_factors",
];
const ATOMIC_MIGRATION_ID: &str = "202607101200_create_atomic_migration_widgets";
const ATOMIC_MIGRATION_TABLE: &str = "foundry_atomic_migration_widgets";
const LATEST_BATCH_MIGRATION_ID: &str = "202607101201_create_latest_batch_widgets";
const LATEST_BATCH_MIGRATION_TABLE: &str = "foundry_latest_batch_widgets";

fn postgres_url() -> Option<String> {
    std::env::var("FOUNDRY_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

async fn lifecycle_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().await
}

fn next_name(prefix: &str) -> String {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{id}")
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn migration_provider() -> GeneratedDatabaseProvider {
    GeneratedDatabaseProvider
}

#[derive(Clone)]
struct GeneratedDatabaseProvider;

#[async_trait]
impl ServiceProvider for GeneratedDatabaseProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        foundry::register_generated_database!(registrar)?;
        Ok(())
    }
}

#[derive(Clone)]
struct AtomicMigrationProvider;

#[async_trait]
impl ServiceProvider for AtomicMigrationProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        foundry::__private::register_generated_migration_file::<AtomicLedgerMigration>(
            registrar,
            MigrationId::new(ATOMIC_MIGRATION_ID),
        )
    }
}

#[derive(Clone)]
struct AtomicAndLatestMigrationProvider;

#[async_trait]
impl ServiceProvider for AtomicAndLatestMigrationProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        foundry::__private::register_generated_migration_file::<AtomicLedgerMigration>(
            registrar,
            MigrationId::new(ATOMIC_MIGRATION_ID),
        )?;
        foundry::__private::register_generated_migration_file::<LatestBatchMigration>(
            registrar,
            MigrationId::new(LATEST_BATCH_MIGRATION_ID),
        )
    }
}

struct AtomicLedgerMigration;

#[async_trait]
impl MigrationFile for AtomicLedgerMigration {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            &format!(
                "CREATE TABLE {} (id BIGINT PRIMARY KEY)",
                quote_identifier(ATOMIC_MIGRATION_TABLE)
            ),
            &[],
        )
        .await?;
        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            &format!("DROP TABLE {}", quote_identifier(ATOMIC_MIGRATION_TABLE)),
            &[],
        )
        .await?;
        Ok(())
    }
}

struct LatestBatchMigration;

#[async_trait]
impl MigrationFile for LatestBatchMigration {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            &format!(
                "CREATE TABLE {} (id BIGINT PRIMARY KEY)",
                quote_identifier(LATEST_BATCH_MIGRATION_TABLE)
            ),
            &[],
        )
        .await?;
        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            &format!(
                "DROP TABLE {}",
                quote_identifier(LATEST_BATCH_MIGRATION_TABLE)
            ),
            &[],
        )
        .await?;
        Ok(())
    }
}

struct TestRuntime {
    _dir: TempDir,
    database: DatabaseManager,
    database_url: String,
    schema: String,
    migration_table: String,
}

impl TestRuntime {
    async fn new() -> Option<Self> {
        let url = postgres_url()?;
        let schema = next_name("foundry_lifecycle_schema");
        let migration_table = next_name("foundry_migrations");
        let dir = tempfile::tempdir().ok()?;

        write_runtime_config(dir.path(), &url, &schema, &migration_table);
        let database = DatabaseManager::from_config(&DatabaseConfig {
            url: url.clone(),
            schema: schema.clone(),
            migration_table: migration_table.clone(),
            ..DatabaseConfig::default()
        })
        .await
        .ok()?;

        let runtime = Self {
            _dir: dir,
            database,
            database_url: url,
            schema,
            migration_table,
        };
        runtime.cleanup().await;
        Some(runtime)
    }

    fn config_dir(&self) -> &Path {
        self._dir.path()
    }

    async fn cleanup(&self) {
        foundry::testing::assert_safe_to_wipe(&self.database_url)
            .expect("test database URL must be explicitly safe to wipe");
        for table in GENERATED_TABLES {
            let _ = self
                .database
                .raw_execute(
                    &format!("DROP TABLE IF EXISTS {} CASCADE", quote_identifier(table)),
                    &[],
                )
                .await;
        }
        let _ = self
            .database
            .raw_execute(
                &format!(
                    "DROP TABLE IF EXISTS {}",
                    quote_identifier(ATOMIC_MIGRATION_TABLE)
                ),
                &[],
            )
            .await;
        let _ = self
            .database
            .raw_execute(
                &format!(
                    "DROP TABLE IF EXISTS {}",
                    quote_identifier(LATEST_BATCH_MIGRATION_TABLE)
                ),
                &[],
            )
            .await;
        let _ = self
            .database
            .raw_execute(
                &format!(
                    "DROP SCHEMA IF EXISTS {} CASCADE",
                    quote_identifier(&self.schema)
                ),
                &[],
            )
            .await;
    }

    async fn table_exists(&self, table: &str) -> bool {
        let records = self
            .database
            .raw_query(
                "SELECT COUNT(*) AS count FROM pg_tables WHERE schemaname = 'public' AND tablename = $1",
                &[table.into()],
            )
            .await
            .unwrap();
        records[0].decode::<i64>("count").unwrap() > 0
    }

    async fn row_count(&self, table: &str) -> i64 {
        let records = self
            .database
            .raw_query(
                &format!("SELECT COUNT(*) AS count FROM {}", quote_identifier(table)),
                &[],
            )
            .await
            .unwrap();
        records[0].decode("count").unwrap()
    }

    async fn applied_migrations(&self) -> Vec<(String, i64)> {
        let records = self
            .database
            .raw_query(
                &format!(
                    "SELECT id, batch FROM {}.{} ORDER BY id",
                    quote_identifier(&self.schema),
                    quote_identifier(&self.migration_table)
                ),
                &[],
            )
            .await
            .unwrap_or_default();

        records
            .into_iter()
            .map(|record| {
                (
                    record.decode::<String>("id").unwrap(),
                    record.decode::<i64>("batch").unwrap(),
                )
            })
            .collect()
    }

    async fn seed_first_generated_migration_manually(&self) {
        self.database
            .raw_execute(
                &format!(
                    "CREATE SCHEMA IF NOT EXISTS {}",
                    quote_identifier(&self.schema)
                ),
                &[],
            )
            .await
            .unwrap();
        self.database
            .raw_execute(
                &format!(
                    "CREATE TABLE IF NOT EXISTS {}.{} (id TEXT PRIMARY KEY, batch BIGINT NOT NULL, applied_at TIMESTAMPTZ NOT NULL)",
                    quote_identifier(&self.schema),
                    quote_identifier(&self.migration_table)
                ),
                &[],
            )
            .await
            .unwrap();
        self.database
            .raw_execute(
                &format!(
                    "INSERT INTO {}.{} (id, batch, applied_at) VALUES ($1, 1, NOW())",
                    quote_identifier(&self.schema),
                    quote_identifier(&self.migration_table)
                ),
                &[FIRST_GENERATED_MIGRATION_ID.into()],
            )
            .await
            .unwrap();
    }

    async fn insert_missing_applied_migration(&self, id: &str) {
        self.database
            .raw_execute(
                &format!(
                    "INSERT INTO {}.{} (id, batch, applied_at) VALUES ($1, 1, NOW())",
                    quote_identifier(&self.schema),
                    quote_identifier(&self.migration_table)
                ),
                &[id.into()],
            )
            .await
            .unwrap();
    }

    async fn create_ledger_rejecting_migration(&self, migration_id: &str) {
        self.database
            .raw_execute(
                &format!(
                    "CREATE SCHEMA IF NOT EXISTS {}",
                    quote_identifier(&self.schema)
                ),
                &[],
            )
            .await
            .unwrap();
        self.database
            .raw_execute(
                &format!(
                    "CREATE TABLE {}.{} (id TEXT PRIMARY KEY, batch BIGINT NOT NULL, applied_at TIMESTAMPTZ NOT NULL, CONSTRAINT {} CHECK (id <> {}))",
                    quote_identifier(&self.schema),
                    quote_identifier(&self.migration_table),
                    quote_identifier("reject_atomic_migration_ledger_insert"),
                    quote_literal(migration_id),
                ),
                &[],
            )
            .await
            .unwrap();
    }

    async fn block_migration_ledger_delete(&self, migration_id: &str) {
        self.database
            .raw_execute(
                &format!(
                    "CREATE TABLE {}.{} (migration_id TEXT PRIMARY KEY, CONSTRAINT {} FOREIGN KEY (migration_id) REFERENCES {}.{} (id))",
                    quote_identifier(&self.schema),
                    quote_identifier("migration_ledger_delete_guard"),
                    quote_identifier("block_atomic_migration_ledger_delete"),
                    quote_identifier(&self.schema),
                    quote_identifier(&self.migration_table),
                ),
                &[],
            )
            .await
            .unwrap();
        self.database
            .raw_execute(
                &format!(
                    "INSERT INTO {}.{} (migration_id) VALUES ($1)",
                    quote_identifier(&self.schema),
                    quote_identifier("migration_ledger_delete_guard"),
                ),
                &[migration_id.into()],
            )
            .await
            .unwrap();
    }
}

fn write_runtime_config(dir: &Path, url: &str, schema: &str, migration_table: &str) {
    fs::write(
        dir.join("00-runtime.toml"),
        format!(
            r#"
            [database]
            url = "{url}"
            schema = "{schema}"
            migration_table = "{migration_table}"
        "#
        ),
    )
    .unwrap();
}

fn write_generator_config(dir: &Path, migrations_path: &Path, seeders_path: &Path) {
    fs::write(
        dir.join("00-runtime.toml"),
        format!(
            r#"
            [database]
            url = ""
            schema = "public"
            migration_table = "foundry_migrations"
            migrations_path = "{}"
            seeders_path = "{}"
        "#,
            migrations_path.display(),
            seeders_path.display()
        ),
    )
    .unwrap();
}

async fn run_cli(builder: AppBuilder, args: Vec<String>) -> Result<()> {
    builder.build_cli_kernel().await?.run_with_args(args).await
}

#[tokio::test]
async fn db_migrate_applies_discovered_rust_migrations_and_records_the_ledger() {
    let _guard = lifecycle_lock().await;
    let Some(runtime) = TestRuntime::new().await else {
        return;
    };

    run_cli(
        App::builder()
            .load_config_dir(runtime.config_dir())
            .register_provider(migration_provider()),
        vec!["foundry".into(), "db:migrate".into()],
    )
    .await
    .unwrap();

    assert!(runtime.table_exists("personal_access_tokens").await);
    assert!(runtime.table_exists("countries").await);
    assert_eq!(
        runtime.applied_migrations().await,
        GENERATED_MIGRATION_IDS
            .iter()
            .map(|id| ((*id).to_string(), 1))
            .collect::<Vec<_>>()
    );

    runtime.cleanup().await;
}

#[tokio::test]
async fn transactional_migration_rolls_back_ddl_when_ledger_insert_fails() {
    let _guard = lifecycle_lock().await;
    let Some(runtime) = TestRuntime::new().await else {
        return;
    };

    runtime
        .create_ledger_rejecting_migration(ATOMIC_MIGRATION_ID)
        .await;

    let error = run_cli(
        App::builder()
            .load_config_dir(runtime.config_dir())
            .register_provider(AtomicMigrationProvider),
        vec!["foundry".into(), "db:migrate".into()],
    )
    .await
    .expect_err("the ledger constraint should reject the migration record");

    assert!(error
        .to_string()
        .contains("reject_atomic_migration_ledger_insert"));
    assert!(!runtime.table_exists(ATOMIC_MIGRATION_TABLE).await);
    assert!(runtime.applied_migrations().await.is_empty());

    runtime.cleanup().await;
}

#[tokio::test]
async fn db_rollback_reverts_only_the_latest_batch() {
    let _guard = lifecycle_lock().await;
    let Some(runtime) = TestRuntime::new().await else {
        return;
    };

    run_cli(
        App::builder()
            .load_config_dir(runtime.config_dir())
            .register_provider(AtomicMigrationProvider),
        vec!["foundry".into(), "db:migrate".into()],
    )
    .await
    .unwrap();

    assert_eq!(
        runtime.applied_migrations().await,
        vec![(ATOMIC_MIGRATION_ID.to_string(), 1)]
    );

    run_cli(
        App::builder()
            .load_config_dir(runtime.config_dir())
            .register_provider(AtomicAndLatestMigrationProvider),
        vec!["foundry".into(), "db:migrate".into()],
    )
    .await
    .unwrap();

    assert_eq!(
        runtime.applied_migrations().await,
        vec![
            (ATOMIC_MIGRATION_ID.to_string(), 1),
            (LATEST_BATCH_MIGRATION_ID.to_string(), 2),
        ]
    );

    run_cli(
        App::builder()
            .load_config_dir(runtime.config_dir())
            .register_provider(AtomicAndLatestMigrationProvider),
        vec!["foundry".into(), "db:rollback".into()],
    )
    .await
    .unwrap();

    assert!(runtime.table_exists(ATOMIC_MIGRATION_TABLE).await);
    assert!(!runtime.table_exists(LATEST_BATCH_MIGRATION_TABLE).await);
    assert_eq!(
        runtime.applied_migrations().await,
        vec![(ATOMIC_MIGRATION_ID.to_string(), 1)]
    );

    runtime.cleanup().await;
}

#[tokio::test]
async fn transactional_rollback_restores_ddl_when_ledger_delete_fails() {
    let _guard = lifecycle_lock().await;
    let Some(runtime) = TestRuntime::new().await else {
        return;
    };

    run_cli(
        App::builder()
            .load_config_dir(runtime.config_dir())
            .register_provider(AtomicMigrationProvider),
        vec!["foundry".into(), "db:migrate".into()],
    )
    .await
    .unwrap();
    runtime
        .block_migration_ledger_delete(ATOMIC_MIGRATION_ID)
        .await;

    let error = run_cli(
        App::builder()
            .load_config_dir(runtime.config_dir())
            .register_provider(AtomicMigrationProvider),
        vec!["foundry".into(), "db:rollback".into()],
    )
    .await
    .expect_err("the ledger foreign key should reject deleting the migration record");

    assert!(error
        .to_string()
        .contains("block_atomic_migration_ledger_delete"));
    assert!(runtime.table_exists(ATOMIC_MIGRATION_TABLE).await);
    assert_eq!(
        runtime.applied_migrations().await,
        vec![(ATOMIC_MIGRATION_ID.to_string(), 1)]
    );

    runtime.cleanup().await;
}

#[tokio::test]
async fn db_migrate_status_reports_missing_applied_migrations_without_failing() {
    let _guard = lifecycle_lock().await;
    let Some(runtime) = TestRuntime::new().await else {
        return;
    };

    runtime.seed_first_generated_migration_manually().await;
    runtime
        .insert_missing_applied_migration("202604101529_removed_migration")
        .await;

    run_cli(
        App::builder()
            .load_config_dir(runtime.config_dir())
            .register_provider(migration_provider()),
        vec![
            "foundry".into(),
            "db:migrate:status".into(),
            "--json".into(),
        ],
    )
    .await
    .unwrap();

    runtime.cleanup().await;
}

#[tokio::test]
async fn db_migrate_fails_clearly_when_applied_migration_is_missing() {
    let _guard = lifecycle_lock().await;
    let Some(runtime) = TestRuntime::new().await else {
        return;
    };

    runtime.seed_first_generated_migration_manually().await;
    runtime
        .insert_missing_applied_migration("202604101529_removed_migration")
        .await;

    let error = run_cli(
        App::builder()
            .load_config_dir(runtime.config_dir())
            .register_provider(migration_provider()),
        vec!["foundry".into(), "db:migrate".into()],
    )
    .await
    .expect_err("migration drift should block db:migrate");
    let message = error.to_string();
    assert!(message.contains("applied migration `202604101529_removed_migration` is missing"));
    assert!(message.contains("db:migrate:status --json"));

    runtime.cleanup().await;
}

#[tokio::test]
async fn db_seed_runs_discovered_seeders_and_honors_id_filtering() {
    let _guard = lifecycle_lock().await;
    let Some(runtime) = TestRuntime::new().await else {
        return;
    };

    run_cli(
        App::builder()
            .load_config_dir(runtime.config_dir())
            .register_provider(migration_provider()),
        vec!["foundry".into(), "db:migrate".into()],
    )
    .await
    .unwrap();

    run_cli(
        App::builder()
            .load_config_dir(runtime.config_dir())
            .register_provider(migration_provider()),
        vec![
            "foundry".into(),
            "db:seed".into(),
            "--id".into(),
            COUNTRIES_SEED_ID.into(),
        ],
    )
    .await
    .unwrap();
    let seeded_countries = runtime.row_count("countries").await;
    assert!(seeded_countries > 200);

    run_cli(
        App::builder()
            .load_config_dir(runtime.config_dir())
            .register_provider(migration_provider()),
        vec!["foundry".into(), "db:seed".into()],
    )
    .await
    .unwrap();
    assert_eq!(runtime.row_count("countries").await, seeded_countries);

    runtime.cleanup().await;
}

#[tokio::test]
async fn make_migration_generates_a_rust_file_and_refuses_overwrite_without_force() {
    let _guard = lifecycle_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let seeders_dir = dir.path().join("seeders");
    write_generator_config(dir.path(), &migrations_dir, &seeders_dir);

    run_cli(
        App::builder().load_config_dir(dir.path()),
        vec![
            "foundry".into(),
            "make:migration".into(),
            "--name".into(),
            "create_widgets".into(),
        ],
    )
    .await
    .unwrap();

    let generated = fs::read_dir(&migrations_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(generated.len(), 1);
    assert!(generated[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .ends_with("_create_widgets.rs"));
    assert!(fs::read_to_string(&generated[0])
        .unwrap()
        .contains("impl MigrationFile for Entry"));

    let error = run_cli(
        App::builder().load_config_dir(dir.path()),
        vec![
            "foundry".into(),
            "make:migration".into(),
            "--name".into(),
            "create_widgets".into(),
        ],
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("refusing to overwrite"));
}

#[tokio::test]
async fn make_seeder_generates_a_rust_file_and_refuses_overwrite_without_force() {
    let _guard = lifecycle_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let seeders_dir = dir.path().join("seeders");
    write_generator_config(dir.path(), &migrations_dir, &seeders_dir);

    run_cli(
        App::builder().load_config_dir(dir.path()),
        vec![
            "foundry".into(),
            "make:seeder".into(),
            "--name".into(),
            "UsersSeed".into(),
        ],
    )
    .await
    .unwrap();

    let generated = fs::read_dir(&seeders_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(generated.len(), 1);
    assert_eq!(
        generated[0].file_name().unwrap().to_string_lossy(),
        "users_seed.rs"
    );
    let generated_seeder = fs::read_to_string(&generated[0]).unwrap();
    assert!(generated_seeder.contains("impl SeederFile for Entry"));
    assert!(generated_seeder.contains("Query::insert_into"));
    assert!(generated_seeder.contains("Sql::uuid_v7()"));

    let error = run_cli(
        App::builder().load_config_dir(dir.path()),
        vec![
            "foundry".into(),
            "make:seeder".into(),
            "--name".into(),
            "UsersSeed".into(),
        ],
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("refusing to overwrite"));
}

#[tokio::test]
async fn make_app_scaffolds_can_target_custom_output_paths() {
    let _guard = lifecycle_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let model_dir = dir.path().join("src/domain/models");
    let job_dir = dir.path().join("src/domain/jobs");
    let command_dir = dir.path().join("src/commands");

    run_cli(
        App::builder(),
        vec![
            "foundry".into(),
            "make:model".into(),
            "--name".into(),
            "AuditEvent".into(),
            "--table".into(),
            "recorded_audits".into(),
            "--path".into(),
            model_dir.display().to_string(),
        ],
    )
    .await
    .unwrap();
    run_cli(
        App::builder(),
        vec![
            "foundry".into(),
            "make:job".into(),
            "--name".into(),
            "SendWelcomeEmail".into(),
            "--path".into(),
            job_dir.display().to_string(),
        ],
    )
    .await
    .unwrap();
    run_cli(
        App::builder(),
        vec![
            "foundry".into(),
            "make:command".into(),
            "--name".into(),
            "SyncInventory".into(),
            "--path".into(),
            command_dir.display().to_string(),
        ],
    )
    .await
    .unwrap();

    let model = fs::read_to_string(model_dir.join("audit_event.rs")).unwrap();
    assert!(model.contains("pub struct AuditEvent"));
    assert!(model.contains("#[foundry(table = \"recorded_audits\")]"));

    let job = fs::read_to_string(job_dir.join("send_welcome_email.rs")).unwrap();
    assert!(job.contains("pub struct SendWelcomeEmail;"));
    assert!(!job.contains("TODO"));

    let command = fs::read_to_string(command_dir.join("sync_inventory.rs")).unwrap();
    assert!(command.contains("pub const SYNC_INVENTORY_COMMAND"));
    assert!(!command.contains("TODO"));
}

#[tokio::test]
async fn make_component_scaffolds_cover_repeated_application_artifacts() {
    let _guard = lifecycle_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("components");
    let cases = [
        ("make:request", "CreateUserRequest", None, None),
        ("make:dto", "UserResponse", None, None),
        ("make:policy", "CanEditUser", None, None),
        ("make:event", "UserCreated", None, None),
        (
            "make:listener",
            "SendWelcomeEmail",
            Some("--event"),
            Some("UserCreated"),
        ),
        ("make:notification", "AccountActivated", None, None),
        ("make:mail", "WelcomeMail", None, None),
        (
            "make:datatable",
            "UsersDatatable",
            Some("--model"),
            Some("User"),
        ),
        ("make:plugin", "AuditToolsPlugin", None, None),
        ("make:test", "UserRegistrationTest", None, None),
    ];

    for (command, name, related_flag, related_value) in cases {
        let mut args = vec![
            "foundry".to_string(),
            command.to_string(),
            "--name".to_string(),
            name.to_string(),
            "--path".to_string(),
            output.display().to_string(),
        ];
        if let (Some(flag), Some(value)) = (related_flag, related_value) {
            args.push(flag.to_string());
            args.push(value.to_string());
        }
        run_cli(App::builder(), args).await.unwrap();
    }

    for filename in [
        "create_user_request.rs",
        "user_response.rs",
        "can_edit_user.rs",
        "user_created.rs",
        "send_welcome_email.rs",
        "account_activated.rs",
        "welcome_mail.rs",
        "users_datatable.rs",
        "audit_tools_plugin.rs",
        "user_registration_test.rs",
    ] {
        let path = output.join(filename);
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.ends_with('\n'), "{filename}");
        assert!(!contents.contains("TODO"), "{filename}");
        let status = std::process::Command::new("rustfmt")
            .arg("--check")
            .args(["--edition", "2024"])
            .arg(&path)
            .status()
            .expect("rustfmt should parse generated component");
        assert!(
            status.success(),
            "generated component is invalid: {filename}"
        );
    }
    assert!(fs::read_to_string(output.join("create_user_request.rs"))
        .unwrap()
        .contains("foundry::Validate"));
    assert!(fs::read_to_string(output.join("send_welcome_email.rs"))
        .unwrap()
        .contains("EventListener<UserCreated>"));
    assert!(fs::read_to_string(output.join("users_datatable.rs"))
        .unwrap()
        .contains("ModelQuery<User>"));
    assert!(fs::read_to_string(output.join("audit_tools_plugin.rs"))
        .unwrap()
        .contains("impl Plugin for AuditToolsPlugin"));
}

#[tokio::test]
async fn migrate_publish_generates_framework_migrations_without_stale_audit_follow_up() {
    let _guard = lifecycle_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let seeders_dir = dir.path().join("seeders");
    let primitives_migration_path =
        migrations_dir.join("000000000000_create_database_primitives.rs");
    let audit_migration_path = migrations_dir.join("000000000010_create_audit_logs.rs");
    let job_history_index_path =
        migrations_dir.join("000000000012_index_job_history_created_at.rs");
    let translation_ids_path =
        migrations_dir.join("000000000013_alter_model_translation_ids_to_text.rs");
    let notification_type_path =
        migrations_dir.join("000000000014_add_notification_notifiable_type.rs");
    let stale_follow_up_path = migrations_dir.join("000000000012_add_area_to_audit_logs.rs");

    write_generator_config(dir.path(), &migrations_dir, &seeders_dir);

    run_cli(
        App::builder().load_config_dir(dir.path()),
        vec![
            "foundry".into(),
            "migrate:publish".into(),
            "--path".into(),
            migrations_dir.display().to_string(),
        ],
    )
    .await
    .unwrap();

    assert!(primitives_migration_path.exists());
    assert!(audit_migration_path.exists());
    assert!(job_history_index_path.exists());
    assert!(translation_ids_path.exists());
    assert!(notification_type_path.exists());
    assert!(!stale_follow_up_path.exists());

    let published_primitives = fs::read_to_string(&primitives_migration_path).unwrap();
    assert!(published_primitives.contains("CREATE EXTENSION IF NOT EXISTS pgcrypto"));
    assert!(published_primitives.contains("CREATE FUNCTION public.uuidv7()"));
    assert!(published_primitives.contains("IF NOT EXISTS"));

    let published = fs::read_to_string(&audit_migration_path).unwrap();
    assert!(published.contains("area TEXT"));
    assert!(published.contains("idx_audit_logs_area_created_at"));

    let published = fs::read_to_string(&job_history_index_path).unwrap();
    assert!(published.contains("idx_job_history_created_at"));

    let published = fs::read_to_string(&translation_ids_path).unwrap();
    assert!(published.contains("ALTER COLUMN translatable_id TYPE TEXT"));

    let published = fs::read_to_string(&notification_type_path).unwrap();
    assert!(published.contains("ADD COLUMN IF NOT EXISTS notifiable_type"));
    assert!(published.contains("idx_notifications_unread"));
}

#[tokio::test]
async fn seed_publish_generates_framework_seeders_and_honors_force() {
    let _guard = lifecycle_lock().await;
    let dir = tempfile::tempdir().unwrap();
    let seeders_dir = dir.path().join("seeders");
    let published_path = seeders_dir.join("000000000001_countries_seeder.rs");

    run_cli(
        App::builder().load_config_dir(dir.path()),
        vec![
            "foundry".into(),
            "seed:publish".into(),
            "--path".into(),
            seeders_dir.display().to_string(),
        ],
    )
    .await
    .unwrap();

    let published = fs::read_to_string(&published_path).unwrap();
    assert!(published.contains("seed_countries_with(ctx)"));
    assert!(published.contains("WHERE iso2 = 'MY'"));
    assert!(published.contains("is_default = true"));
    assert!(published.contains("status = 'enabled'"));

    fs::write(&published_path, "// custom seeder").unwrap();

    run_cli(
        App::builder().load_config_dir(dir.path()),
        vec![
            "foundry".into(),
            "seed:publish".into(),
            "--path".into(),
            seeders_dir.display().to_string(),
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        fs::read_to_string(&published_path).unwrap(),
        "// custom seeder"
    );

    run_cli(
        App::builder().load_config_dir(dir.path()),
        vec![
            "foundry".into(),
            "seed:publish".into(),
            "--path".into(),
            seeders_dir.display().to_string(),
            "--force".into(),
        ],
    )
    .await
    .unwrap();
    assert!(fs::read_to_string(&published_path)
        .unwrap()
        .contains("status = 'enabled'"));
}
