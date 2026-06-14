use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use async_trait::async_trait;
use foundry::config::DatabaseConfig;
use foundry::prelude::*;
use tempfile::TempDir;
use tokio::sync::{Mutex, MutexGuard};

const CREATE_USERS_MIGRATION_ID: &str = "202604101530_create_users";
const CREATE_PROFILES_MIGRATION_ID: &str = "202604101531_create_profiles";
const USERS_SEED_ID: &str = "users_seed";
const USERS_TABLE: &str = "users";
const PROFILES_TABLE: &str = "profiles";

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
        let _ = self
            .database
            .raw_execute(
                &format!("DROP TABLE IF EXISTS {}", quote_identifier(PROFILES_TABLE)),
                &[],
            )
            .await;
        let _ = self
            .database
            .raw_execute(
                &format!("DROP TABLE IF EXISTS {}", quote_identifier(USERS_TABLE)),
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

    async fn seed_first_batch_manually(&self) {
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
                    "CREATE TABLE IF NOT EXISTS {} (id UUID PRIMARY KEY DEFAULT uuidv7(), email TEXT NOT NULL UNIQUE, created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(), updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW())",
                    quote_identifier(USERS_TABLE)
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
                &[CREATE_USERS_MIGRATION_ID.into()],
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

    assert!(runtime.table_exists(USERS_TABLE).await);
    assert!(runtime.table_exists(PROFILES_TABLE).await);
    assert_eq!(
        runtime.applied_migrations().await,
        vec![
            (CREATE_USERS_MIGRATION_ID.to_string(), 1),
            (CREATE_PROFILES_MIGRATION_ID.to_string(), 1),
        ]
    );

    runtime.cleanup().await;
}

#[tokio::test]
async fn db_rollback_reverts_only_the_latest_generated_batch() {
    let _guard = lifecycle_lock().await;
    let Some(runtime) = TestRuntime::new().await else {
        return;
    };

    runtime.seed_first_batch_manually().await;

    run_cli(
        App::builder()
            .load_config_dir(runtime.config_dir())
            .register_provider(migration_provider()),
        vec!["foundry".into(), "db:migrate".into()],
    )
    .await
    .unwrap();

    assert_eq!(
        runtime.applied_migrations().await,
        vec![
            (CREATE_USERS_MIGRATION_ID.to_string(), 1),
            (CREATE_PROFILES_MIGRATION_ID.to_string(), 2),
        ]
    );

    run_cli(
        App::builder()
            .load_config_dir(runtime.config_dir())
            .register_provider(migration_provider()),
        vec!["foundry".into(), "db:rollback".into()],
    )
    .await
    .unwrap();

    assert!(runtime.table_exists(USERS_TABLE).await);
    assert!(!runtime.table_exists(PROFILES_TABLE).await);
    assert_eq!(
        runtime.applied_migrations().await,
        vec![(CREATE_USERS_MIGRATION_ID.to_string(), 1)]
    );

    runtime.cleanup().await;
}

#[tokio::test]
async fn db_migrate_status_reports_missing_applied_migrations_without_failing() {
    let _guard = lifecycle_lock().await;
    let Some(runtime) = TestRuntime::new().await else {
        return;
    };

    runtime.seed_first_batch_manually().await;
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

    runtime.seed_first_batch_manually().await;
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
            USERS_SEED_ID.into(),
        ],
    )
    .await
    .unwrap();
    assert_eq!(runtime.row_count(USERS_TABLE).await, 1);

    run_cli(
        App::builder()
            .load_config_dir(runtime.config_dir())
            .register_provider(migration_provider()),
        vec!["foundry".into(), "db:seed".into()],
    )
    .await
    .unwrap();
    assert_eq!(runtime.row_count(USERS_TABLE).await, 2);

    runtime.cleanup().await;
}

#[tokio::test]
async fn make_migration_generates_a_rust_file_and_refuses_overwrite_without_force() {
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

    let job = fs::read_to_string(job_dir.join("send_welcome_email.rs")).unwrap();
    assert!(job.contains("pub struct SendWelcomeEmail;"));
    assert!(!job.contains("TODO"));

    let command = fs::read_to_string(command_dir.join("sync_inventory.rs")).unwrap();
    assert!(command.contains("pub const SYNC_INVENTORY_COMMAND"));
    assert!(!command.contains("TODO"));
}

#[tokio::test]
async fn migrate_publish_generates_framework_migrations_without_stale_audit_follow_up() {
    let dir = tempfile::tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let seeders_dir = dir.path().join("seeders");
    let primitives_migration_path =
        migrations_dir.join("000000000000_create_database_primitives.rs");
    let audit_migration_path = migrations_dir.join("000000000010_create_audit_logs.rs");
    let job_history_index_path =
        migrations_dir.join("000000000012_index_job_history_created_at.rs");
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
}

#[tokio::test]
async fn seed_publish_generates_framework_seeders_and_honors_force() {
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
