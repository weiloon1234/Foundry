use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use clap::{Arg, ArgAction, Command};
use foundry_build::{discover_migration_sources, discover_seeder_sources};
use serde::Serialize;
use tokio::time::{sleep, Duration, Instant};

use crate::cli::{CommandInvocation, CommandRegistrar};
use crate::config::DatabaseConfig;
use crate::foundation::{AppContext, Error, Result};
use crate::logging::{catch_async_panic, catch_sync_panic, panic_payload_message};
use crate::support::sync::lock_unpoisoned;
use crate::support::{CommandId, MigrationId, SeederId};

use super::runtime::{DatabaseSession, QueryExecutionOptions, QueryExecutor};
use super::{DatabaseManager, DbRecord, DbValue};

const DB_MIGRATE_COMMAND: CommandId = CommandId::new("db:migrate");
const DB_MIGRATE_STATUS_COMMAND: CommandId = CommandId::new("db:migrate:status");
const DB_ROLLBACK_COMMAND: CommandId = CommandId::new("db:rollback");
const DB_SEED_COMMAND: CommandId = CommandId::new("db:seed");
const MIGRATION_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(200);
const MIGRATION_LOCK_NOTICE_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppliedMigration {
    pub id: MigrationId,
    pub batch: i64,
    pub applied_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MigrationStatus {
    pub id: MigrationId,
    pub applied: Option<AppliedMigration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MigrationStatusReport {
    pub statuses: Vec<MigrationStatus>,
    pub missing_applied: Vec<AppliedMigration>,
    pub latest_batch: Option<i64>,
}

impl MigrationStatusReport {
    fn summary(&self) -> MigrationStatusSummary {
        let registered = self.statuses.len();
        let applied = self
            .statuses
            .iter()
            .filter(|status| status.applied.is_some())
            .count();
        let pending = registered.saturating_sub(applied);
        let missing_applied = self.missing_applied.len();
        MigrationStatusSummary {
            registered,
            applied,
            pending,
            missing_applied,
            drifted: missing_applied > 0,
            up_to_date: pending == 0 && missing_applied == 0,
            latest_batch: self.latest_batch,
        }
    }

    fn to_json(&self) -> MigrationStatusJson {
        MigrationStatusJson {
            summary: self.summary(),
            migrations: self
                .statuses
                .iter()
                .map(MigrationStatusJsonRow::from_status)
                .collect(),
            missing_applied: self
                .missing_applied
                .iter()
                .map(MigrationStatusJsonRow::from_missing)
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(crate) struct MigrationStatusSummary {
    pub(crate) registered: usize,
    pub(crate) applied: usize,
    pub(crate) pending: usize,
    pub(crate) missing_applied: usize,
    pub(crate) drifted: bool,
    pub(crate) up_to_date: bool,
    pub(crate) latest_batch: Option<i64>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct MigrationStatusJson {
    summary: MigrationStatusSummary,
    migrations: Vec<MigrationStatusJsonRow>,
    missing_applied: Vec<MigrationStatusJsonRow>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
struct MigrationStatusJsonRow {
    id: String,
    state: &'static str,
    batch: Option<i64>,
    applied_at: Option<String>,
}

impl MigrationStatusJsonRow {
    fn from_status(status: &MigrationStatus) -> Self {
        match &status.applied {
            Some(applied) => Self {
                id: status.id.to_string(),
                state: "applied",
                batch: Some(applied.batch),
                applied_at: Some(applied.applied_at.clone()),
            },
            None => Self {
                id: status.id.to_string(),
                state: "pending",
                batch: None,
                applied_at: None,
            },
        }
    }

    fn from_missing(applied: &AppliedMigration) -> Self {
        Self {
            id: applied.id.to_string(),
            state: "missing_applied",
            batch: Some(applied.batch),
            applied_at: Some(applied.applied_at.clone()),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct GeneratedDatabasePaths {
    migration_dirs: Vec<PathBuf>,
    seeder_dirs: Vec<PathBuf>,
}

impl GeneratedDatabasePaths {
    pub(crate) fn new(migration_dirs: Vec<PathBuf>, seeder_dirs: Vec<PathBuf>) -> Self {
        Self {
            migration_dirs,
            seeder_dirs,
        }
    }

    pub(crate) fn migration_dirs(&self) -> &[PathBuf] {
        &self.migration_dirs
    }

    pub(crate) fn seeder_dirs(&self) -> &[PathBuf] {
        &self.seeder_dirs
    }

    pub(crate) fn primary_migration_dir(&self) -> Option<&Path> {
        self.migration_dirs.first().map(PathBuf::as_path)
    }

    pub(crate) fn primary_seeder_dir(&self) -> Option<&Path> {
        self.seeder_dirs.first().map(PathBuf::as_path)
    }
}

pub struct MigrationContext<'a> {
    app: &'a AppContext,
    database: &'a DatabaseManager,
    executor: &'a dyn QueryExecutor,
}

impl<'a> MigrationContext<'a> {
    fn new(
        app: &'a AppContext,
        database: &'a DatabaseManager,
        executor: &'a dyn QueryExecutor,
    ) -> Self {
        Self {
            app,
            database,
            executor,
        }
    }

    pub fn app(&self) -> &AppContext {
        self.app
    }

    pub fn database(&self) -> &DatabaseManager {
        self.database
    }

    pub fn executor(&self) -> &dyn QueryExecutor {
        self.executor
    }

    pub async fn raw_query(&self, sql: &str, bindings: &[DbValue]) -> Result<Vec<DbRecord>> {
        self.executor.raw_query(sql, bindings).await
    }

    pub async fn raw_execute(&self, sql: &str, bindings: &[DbValue]) -> Result<u64> {
        self.executor.raw_execute(sql, bindings).await
    }
}

#[async_trait]
impl QueryExecutor for MigrationContext<'_> {
    async fn raw_query_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<Vec<DbRecord>> {
        self.executor.raw_query_with(sql, bindings, options).await
    }

    async fn raw_execute_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<u64> {
        self.executor.raw_execute_with(sql, bindings, options).await
    }
}

pub struct SeederContext<'a> {
    app: &'a AppContext,
    database: &'a DatabaseManager,
    executor: &'a dyn QueryExecutor,
}

impl<'a> SeederContext<'a> {
    fn new(
        app: &'a AppContext,
        database: &'a DatabaseManager,
        executor: &'a dyn QueryExecutor,
    ) -> Self {
        Self {
            app,
            database,
            executor,
        }
    }

    pub fn app(&self) -> &AppContext {
        self.app
    }

    pub fn database(&self) -> &DatabaseManager {
        self.database
    }

    pub fn executor(&self) -> &dyn QueryExecutor {
        self.executor
    }

    pub async fn raw_query(&self, sql: &str, bindings: &[DbValue]) -> Result<Vec<DbRecord>> {
        self.executor.raw_query(sql, bindings).await
    }

    pub async fn raw_execute(&self, sql: &str, bindings: &[DbValue]) -> Result<u64> {
        self.executor.raw_execute(sql, bindings).await
    }
}

#[async_trait]
impl QueryExecutor for SeederContext<'_> {
    async fn raw_query_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<Vec<DbRecord>> {
        self.executor.raw_query_with(sql, bindings, options).await
    }

    async fn raw_execute_with(
        &self,
        sql: &str,
        bindings: &[DbValue],
        options: QueryExecutionOptions,
    ) -> Result<u64> {
        self.executor.raw_execute_with(sql, bindings, options).await
    }
}

#[async_trait]
pub trait MigrationFile: Send + Sync + 'static {
    fn run_in_transaction() -> bool {
        true
    }

    async fn up(ctx: &MigrationContext<'_>) -> Result<()>;

    async fn down(ctx: &MigrationContext<'_>) -> Result<()>;
}

#[async_trait]
pub trait SeederFile: Send + Sync + 'static {
    fn run_in_transaction() -> bool {
        true
    }

    async fn run(ctx: &SeederContext<'_>) -> Result<()>;
}

#[async_trait]
trait DynMigration: Send + Sync {
    fn id(&self) -> MigrationId;

    fn run_in_transaction(&self) -> bool;

    async fn up(
        &self,
        app: &AppContext,
        database: &DatabaseManager,
        executor: &dyn QueryExecutor,
    ) -> Result<()>;

    async fn down(
        &self,
        app: &AppContext,
        database: &DatabaseManager,
        executor: &dyn QueryExecutor,
    ) -> Result<()>;
}

#[async_trait]
trait DynSeeder: Send + Sync {
    fn id(&self) -> SeederId;

    fn run_in_transaction(&self) -> bool;

    async fn run(
        &self,
        app: &AppContext,
        database: &DatabaseManager,
        executor: &dyn QueryExecutor,
    ) -> Result<()>;
}

struct MigrationFileAdapter<M> {
    id: MigrationId,
    marker: std::marker::PhantomData<M>,
}

#[async_trait]
impl<M> DynMigration for MigrationFileAdapter<M>
where
    M: MigrationFile,
{
    fn id(&self) -> MigrationId {
        self.id.clone()
    }

    fn run_in_transaction(&self) -> bool {
        M::run_in_transaction()
    }

    async fn up(
        &self,
        app: &AppContext,
        database: &DatabaseManager,
        executor: &dyn QueryExecutor,
    ) -> Result<()> {
        let context = MigrationContext::new(app, database, executor);
        M::up(&context).await
    }

    async fn down(
        &self,
        app: &AppContext,
        database: &DatabaseManager,
        executor: &dyn QueryExecutor,
    ) -> Result<()> {
        let context = MigrationContext::new(app, database, executor);
        M::down(&context).await
    }
}

struct SeederFileAdapter<S> {
    id: SeederId,
    marker: std::marker::PhantomData<S>,
}

#[async_trait]
impl<S> DynSeeder for SeederFileAdapter<S>
where
    S: SeederFile,
{
    fn id(&self) -> SeederId {
        self.id.clone()
    }

    fn run_in_transaction(&self) -> bool {
        S::run_in_transaction()
    }

    async fn run(
        &self,
        app: &AppContext,
        database: &DatabaseManager,
        executor: &dyn QueryExecutor,
    ) -> Result<()> {
        let context = SeederContext::new(app, database, executor);
        S::run(&context).await
    }
}

pub(crate) type MigrationRegistryHandle = Arc<Mutex<MigrationRegistryBuilder>>;
pub(crate) type SeederRegistryHandle = Arc<Mutex<SeederRegistryBuilder>>;

#[derive(Default)]
pub(crate) struct MigrationRegistryBuilder {
    migrations: BTreeMap<MigrationId, Arc<dyn DynMigration>>,
}

impl MigrationRegistryBuilder {
    pub(crate) fn shared() -> MigrationRegistryHandle {
        Arc::new(Mutex::new(Self::default()))
    }

    pub(crate) fn register_file<M>(&mut self, id: MigrationId) -> Result<()>
    where
        M: MigrationFile,
    {
        if self.migrations.contains_key(&id) {
            return Err(Error::message(format!(
                "migration `{id}` already registered"
            )));
        }

        self.migrations.insert(
            id.clone(),
            Arc::new(MigrationFileAdapter::<M> {
                id,
                marker: std::marker::PhantomData,
            }),
        );
        Ok(())
    }

    pub(crate) fn freeze_shared(handle: MigrationRegistryHandle) -> Result<MigrationRegistry> {
        let builder = lock_unpoisoned(&handle, "migration registry");
        Ok(MigrationRegistry {
            migrations: builder.migrations.values().cloned().collect(),
        })
    }
}

#[derive(Default)]
pub(crate) struct SeederRegistryBuilder {
    seeders: Vec<Arc<dyn DynSeeder>>,
    ids: HashSet<SeederId>,
}

impl SeederRegistryBuilder {
    pub(crate) fn shared() -> SeederRegistryHandle {
        Arc::new(Mutex::new(Self::default()))
    }

    pub(crate) fn register_file<S>(&mut self, id: SeederId) -> Result<()>
    where
        S: SeederFile,
    {
        if !self.ids.insert(id.clone()) {
            return Err(Error::message(format!("seeder `{id}` already registered")));
        }

        self.seeders.push(Arc::new(SeederFileAdapter::<S> {
            id,
            marker: std::marker::PhantomData,
        }));
        Ok(())
    }

    pub(crate) fn freeze_shared(handle: SeederRegistryHandle) -> Result<SeederRegistry> {
        let mut builder = lock_unpoisoned(&handle, "seeder registry");
        Ok(SeederRegistry {
            seeders: std::mem::take(&mut builder.seeders),
        })
    }
}

pub(crate) struct MigrationRegistry {
    migrations: Vec<Arc<dyn DynMigration>>,
}

impl MigrationRegistry {
    pub(crate) fn ids(&self) -> Vec<MigrationId> {
        self.migrations
            .iter()
            .map(|migration| migration.id())
            .collect()
    }

    pub(crate) fn contains(&self, id: &MigrationId) -> bool {
        self.migrations
            .iter()
            .any(|migration| migration.id() == *id)
    }

    fn migration(&self, id: &MigrationId) -> Option<&Arc<dyn DynMigration>> {
        self.migrations
            .iter()
            .find(|migration| migration.id() == *id)
    }

    fn entries(&self) -> &[Arc<dyn DynMigration>] {
        &self.migrations
    }
}

pub(crate) struct SeederRegistry {
    seeders: Vec<Arc<dyn DynSeeder>>,
}

impl SeederRegistry {
    pub(crate) fn ids(&self) -> Vec<SeederId> {
        self.seeders.iter().map(|seeder| seeder.id()).collect()
    }

    pub(crate) fn contains(&self, id: &SeederId) -> bool {
        self.seeders.iter().any(|seeder| seeder.id() == *id)
    }

    fn entries(&self) -> &[Arc<dyn DynSeeder>] {
        &self.seeders
    }
}

pub(crate) fn builtin_cli_registrar() -> CommandRegistrar {
    Arc::new(|registry| {
        registry.command(
            DB_MIGRATE_COMMAND,
            Command::new(DB_MIGRATE_COMMAND.as_str().to_string())
                .about("Apply pending Foundry database migrations")
                .arg(lock_timeout_arg()),
            |invocation| async move { db_migrate_command(invocation).await },
        )?;
        registry.command(
            DB_MIGRATE_STATUS_COMMAND,
            Command::new(DB_MIGRATE_STATUS_COMMAND.as_str().to_string())
                .about("Show the current Foundry database migration status")
                .arg(
                    Arg::new("json")
                        .long("json")
                        .action(ArgAction::SetTrue)
                        .help("Print migration status as JSON"),
                ),
            |invocation| async move { db_migrate_status_command(invocation).await },
        )?;
        registry.command(
            DB_ROLLBACK_COMMAND,
            Command::new(DB_ROLLBACK_COMMAND.as_str().to_string())
                .about("Rollback the latest Foundry migration batch")
                .arg(lock_timeout_arg()),
            |invocation| async move { db_rollback_command(invocation).await },
        )?;
        registry.command(
            DB_SEED_COMMAND,
            Command::new(DB_SEED_COMMAND.as_str().to_string())
                .about("Run registered Foundry database seeders")
                .arg(
                    Arg::new("id")
                        .long("id")
                        .value_name("SEEDER_ID")
                        .action(ArgAction::Append)
                        .help("Run a specific seeder id; repeat to run more than one"),
                ),
            |invocation| async move { db_seed_command(invocation).await },
        )?;
        Ok(())
    })
}

fn lock_timeout_arg() -> Arg {
    Arg::new("lock_timeout_ms")
        .long("lock-timeout-ms")
        .value_name("MS")
        .value_parser(clap::value_parser!(u64))
        .help("Override database.migration_lock_timeout_ms for this command")
}

async fn db_migrate_command(invocation: CommandInvocation) -> Result<()> {
    let lifecycle = DatabaseLifecycle::from_app(invocation.app())?;
    let lock_timeout_ms = lock_timeout_ms(&invocation, &lifecycle.config);
    let summary = lifecycle.migrate(lock_timeout_ms).await?;
    match summary.batch {
        Some(batch) => println!("applied {} migration(s) in batch {}", summary.count, batch),
        None => println!("applied 0 migration(s)"),
    }
    Ok(())
}

async fn db_migrate_status_command(invocation: CommandInvocation) -> Result<()> {
    let lifecycle = DatabaseLifecycle::from_app(invocation.app())?;
    let report = lifecycle.status_report().await?;
    if invocation.matches().get_flag("json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&report.to_json()).map_err(Error::other)?
        );
        return Ok(());
    }

    for status in report.statuses {
        match status.applied {
            Some(applied) => println!(
                "{} | Applied | batch {} | {}",
                status.id, applied.batch, applied.applied_at
            ),
            None => println!("{} | Pending", status.id),
        }
    }
    for applied in report.missing_applied {
        println!(
            "{} | MissingApplied | batch {} | {}",
            applied.id, applied.batch, applied.applied_at
        );
    }
    Ok(())
}

async fn db_rollback_command(invocation: CommandInvocation) -> Result<()> {
    let lifecycle = DatabaseLifecycle::from_app(invocation.app())?;
    let lock_timeout_ms = lock_timeout_ms(&invocation, &lifecycle.config);
    let summary = lifecycle.rollback_latest_batch(lock_timeout_ms).await?;
    match summary.batch {
        Some(batch) => println!(
            "reverted {} migration(s) from batch {}",
            summary.count, batch
        ),
        None => println!("reverted 0 migration(s)"),
    }
    Ok(())
}

fn lock_timeout_ms(invocation: &CommandInvocation, config: &DatabaseConfig) -> u64 {
    invocation
        .matches()
        .get_one::<u64>("lock_timeout_ms")
        .copied()
        .unwrap_or(config.migration_lock_timeout_ms)
}

async fn db_seed_command(invocation: CommandInvocation) -> Result<()> {
    let lifecycle = DatabaseLifecycle::from_app(invocation.app())?;
    let selected_ids = invocation.matches().get_many::<String>("id").map(|values| {
        values
            .map(|value| SeederId::owned(value.to_string()))
            .collect::<BTreeSet<_>>()
    });
    let count = lifecycle.seed(selected_ids).await?;
    println!("ran {} seeder(s)", count);
    Ok(())
}

struct DatabaseLifecycle {
    app: AppContext,
    database: Arc<DatabaseManager>,
    config: DatabaseConfig,
    migrations: Arc<MigrationRegistry>,
    seeders: Arc<SeederRegistry>,
    generated_paths: Option<Arc<GeneratedDatabasePaths>>,
}

impl DatabaseLifecycle {
    fn from_app(app: &AppContext) -> Result<Self> {
        let database = app.database()?;
        if !database.is_configured() {
            return Err(Error::message("database is not configured"));
        }

        Ok(Self {
            app: app.clone(),
            config: app.config().database()?,
            database,
            migrations: app.resolve::<MigrationRegistry>()?,
            seeders: app.resolve::<SeederRegistry>()?,
            generated_paths: app.resolve::<GeneratedDatabasePaths>().ok(),
        })
    }

    async fn status_report(&self) -> Result<MigrationStatusReport> {
        self.ensure_generated_database_is_registered()?;
        let session = self.database.acquire_session().await?;
        ensure_ledger_table(&self.config, &session).await?;
        let applied = applied_migrations(&self.config, &session).await?;
        let statuses = self
            .migrations
            .ids()
            .into_iter()
            .map(|id| MigrationStatus {
                applied: applied.get(&id).cloned(),
                id,
            })
            .collect::<Vec<_>>();
        let missing_applied = missing_applied_migrations(&applied, &self.migrations);
        let latest_batch = applied.values().map(|migration| migration.batch).max();

        Ok(MigrationStatusReport {
            statuses,
            missing_applied,
            latest_batch,
        })
    }

    async fn migrate(&self, lock_timeout_ms: u64) -> Result<MigrationRunSummary> {
        self.ensure_generated_database_is_registered()?;
        let session = self.database.acquire_session().await?;
        let lock_key = advisory_lock_key(&self.config);
        acquire_migration_lock(&session, &self.config, lock_key, lock_timeout_ms).await?;
        let result = migrate_locked(
            self.app.clone(),
            self.database.clone(),
            self.config.clone(),
            self.migrations.clone(),
            &session,
        )
        .await;
        finish_locked_operation(&session, lock_key, result).await
    }

    async fn rollback_latest_batch(&self, lock_timeout_ms: u64) -> Result<MigrationRunSummary> {
        self.ensure_generated_database_is_registered()?;
        let session = self.database.acquire_session().await?;
        let lock_key = advisory_lock_key(&self.config);
        acquire_migration_lock(&session, &self.config, lock_key, lock_timeout_ms).await?;
        let result = rollback_locked(
            self.app.clone(),
            self.database.clone(),
            self.config.clone(),
            self.migrations.clone(),
            &session,
        )
        .await;
        finish_locked_operation(&session, lock_key, result).await
    }

    async fn seed(&self, selected_ids: Option<BTreeSet<SeederId>>) -> Result<usize> {
        self.ensure_generated_database_is_registered()?;
        if let Some(selected_ids) = &selected_ids {
            for id in selected_ids {
                if !self.seeders.contains(id) {
                    return Err(Error::message(format!("seeder `{id}` is not registered")));
                }
            }
        }

        let session = self.database.acquire_session().await?;
        let mut ran = 0usize;
        for seeder in self.seeders.entries() {
            if selected_ids
                .as_ref()
                .is_some_and(|selected| !selected.contains(&seeder.id()))
            {
                continue;
            }

            run_seeder(
                self.app.clone(),
                self.database.clone(),
                &session,
                seeder.as_ref(),
            )
            .await?;
            ran += 1;
        }

        Ok(ran)
    }
    fn ensure_generated_database_is_registered(&self) -> Result<()> {
        let Some(paths) = &self.generated_paths else {
            return Ok(());
        };

        let migration_ids = self
            .migrations
            .ids()
            .into_iter()
            .map(|id| id.to_string())
            .collect::<HashSet<_>>();
        for source in discover_migration_sources(paths.migration_dirs()).map_err(Error::other)? {
            if !migration_ids.contains(&source.id) {
                return Err(Error::message(format!(
                    "migration file `{}` exists but is not registered in the current binary; rebuild the app before running database lifecycle commands so the file is discovered",
                    source.path.display()
                )));
            }
        }

        let seeder_ids = self
            .seeders
            .ids()
            .into_iter()
            .map(|id| id.to_string())
            .collect::<HashSet<_>>();
        for source in discover_seeder_sources(paths.seeder_dirs()).map_err(Error::other)? {
            if !seeder_ids.contains(&source.id) {
                return Err(Error::message(format!(
                    "seeder file `{}` exists but is not registered in the current binary; rebuild the app before running database lifecycle commands so the file is discovered",
                    source.path.display()
                )));
            }
        }

        Ok(())
    }
}

pub(crate) async fn migration_status_summary_from_app(
    app: &AppContext,
) -> Result<MigrationStatusSummary> {
    DatabaseLifecycle::from_app(app)?
        .status_report()
        .await
        .map(|report| report.summary())
}

struct MigrationRunSummary {
    count: usize,
    batch: Option<i64>,
}

#[async_trait]
trait MigrationLockClient: Sync {
    async fn try_acquire_migration_lock(&self, lock_key: i64) -> Result<bool>;
}

#[async_trait]
impl MigrationLockClient for DatabaseSession {
    async fn try_acquire_migration_lock(&self, lock_key: i64) -> Result<bool> {
        self.try_acquire_advisory_lock(lock_key).await
    }
}

async fn acquire_migration_lock(
    client: &dyn MigrationLockClient,
    config: &DatabaseConfig,
    lock_key: i64,
    timeout_ms: u64,
) -> Result<()> {
    let started = Instant::now();
    let timeout = (timeout_ms > 0).then(|| Duration::from_millis(timeout_ms));
    let mut next_notice = started + MIGRATION_LOCK_NOTICE_INTERVAL;

    loop {
        if client.try_acquire_migration_lock(lock_key).await? {
            return Ok(());
        }

        let now = Instant::now();
        if let Some(timeout) = timeout {
            if now.duration_since(started) >= timeout {
                return Err(migration_lock_timeout_error(config, timeout_ms));
            }
        }

        if now >= next_notice {
            let message = format!(
                "waiting for migration advisory lock for schema `{}` and migration table `{}`; another db:migrate or db:rollback process is likely running",
                config.schema, config.migration_table
            );
            tracing::warn!(target: "foundry.database.lifecycle", "{message}");
            println!("{message}");
            next_notice = now + MIGRATION_LOCK_NOTICE_INTERVAL;
        }

        let sleep_for = timeout
            .map(|timeout| timeout.saturating_sub(now.duration_since(started)))
            .map(|remaining| remaining.min(MIGRATION_LOCK_POLL_INTERVAL))
            .unwrap_or(MIGRATION_LOCK_POLL_INTERVAL);
        if sleep_for.is_zero() {
            return Err(migration_lock_timeout_error(config, timeout_ms));
        }
        sleep(sleep_for).await;
    }
}

fn migration_lock_timeout_error(config: &DatabaseConfig, timeout_ms: u64) -> Error {
    Error::message(format!(
        "timed out after {timeout_ms}ms waiting for migration advisory lock for schema `{}` and migration table `{}`; another db:migrate or db:rollback process is likely still running",
        config.schema, config.migration_table
    ))
}

async fn finish_locked_operation(
    session: &DatabaseSession,
    lock_key: i64,
    result: Result<MigrationRunSummary>,
) -> Result<MigrationRunSummary> {
    match (result, session.release_advisory_lock(lock_key).await) {
        (Ok(summary), Ok(())) => Ok(summary),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(unlock_error)) => Err(unlock_error),
        (Err(error), Err(unlock_error)) => Err(Error::message(format!(
            "{error}; advisory unlock failed: {unlock_error}"
        ))),
    }
}

async fn migrate_locked(
    app: AppContext,
    database: Arc<DatabaseManager>,
    config: DatabaseConfig,
    migrations: Arc<MigrationRegistry>,
    session: &DatabaseSession,
) -> Result<MigrationRunSummary> {
    ensure_ledger_table(&config, session).await?;
    let applied = applied_migrations(&config, session).await?;
    ensure_applied_migrations_exist(&applied, &migrations)?;

    let pending = migrations
        .entries()
        .iter()
        .filter(|migration| !applied.contains_key(&migration.id()))
        .cloned()
        .collect::<Vec<_>>();

    if pending.is_empty() {
        return Ok(MigrationRunSummary {
            count: 0,
            batch: None,
        });
    }

    let next_batch = applied
        .values()
        .map(|migration| migration.batch)
        .max()
        .unwrap_or(0)
        + 1;

    for migration in &pending {
        run_migration_up(app.clone(), database.clone(), session, migration.as_ref()).await?;
        record_applied_migration(session, &config, &migration.id(), next_batch).await?;
    }

    Ok(MigrationRunSummary {
        count: pending.len(),
        batch: Some(next_batch),
    })
}

async fn rollback_locked(
    app: AppContext,
    database: Arc<DatabaseManager>,
    config: DatabaseConfig,
    migrations: Arc<MigrationRegistry>,
    session: &DatabaseSession,
) -> Result<MigrationRunSummary> {
    ensure_ledger_table(&config, session).await?;
    let applied = applied_migrations(&config, session).await?;
    ensure_applied_migrations_exist(&applied, &migrations)?;

    let latest_batch = applied.values().map(|migration| migration.batch).max();
    let Some(latest_batch) = latest_batch else {
        return Ok(MigrationRunSummary {
            count: 0,
            batch: None,
        });
    };

    let rollback = migrations
        .ids()
        .into_iter()
        .rev()
        .filter_map(|id| {
            applied
                .get(&id)
                .filter(|migration| migration.batch == latest_batch)
                .and_then(|_| migrations.migration(&id).cloned())
        })
        .collect::<Vec<_>>();

    for migration in &rollback {
        run_migration_down(app.clone(), database.clone(), session, migration.as_ref()).await?;
        delete_applied_migration(session, &config, &migration.id()).await?;
    }

    Ok(MigrationRunSummary {
        count: rollback.len(),
        batch: Some(latest_batch),
    })
}

async fn run_migration_up(
    app: AppContext,
    database: Arc<DatabaseManager>,
    session: &DatabaseSession,
    migration: &dyn DynMigration,
) -> Result<()> {
    let id = migration.id();
    if !migration_run_in_transaction(migration, &id)? {
        return run_database_lifecycle_callback("migration", &id, "up", || {
            migration.up(&app, &database, session)
        })
        .await;
    }

    session.begin_transaction().await?;
    let result = run_database_lifecycle_callback("migration", &id, "up", || {
        migration.up(&app, &database, session)
    })
    .await;
    match result {
        Ok(()) => session.commit_transaction().await,
        Err(error) => match session.rollback_transaction().await {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(Error::message(format!(
                "{error}; rollback failed: {rollback_error}"
            ))),
        },
    }
}

async fn run_migration_down(
    app: AppContext,
    database: Arc<DatabaseManager>,
    session: &DatabaseSession,
    migration: &dyn DynMigration,
) -> Result<()> {
    let id = migration.id();
    if !migration_run_in_transaction(migration, &id)? {
        return run_database_lifecycle_callback("migration", &id, "down", || {
            migration.down(&app, &database, session)
        })
        .await;
    }

    session.begin_transaction().await?;
    let result = run_database_lifecycle_callback("migration", &id, "down", || {
        migration.down(&app, &database, session)
    })
    .await;
    match result {
        Ok(()) => session.commit_transaction().await,
        Err(error) => match session.rollback_transaction().await {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(Error::message(format!(
                "{error}; rollback failed: {rollback_error}"
            ))),
        },
    }
}

async fn run_seeder(
    app: AppContext,
    database: Arc<DatabaseManager>,
    session: &DatabaseSession,
    seeder: &dyn DynSeeder,
) -> Result<()> {
    let id = seeder.id();
    if !seeder_run_in_transaction(seeder, &id)? {
        return run_database_lifecycle_callback("seeder", &id, "run", || {
            seeder.run(&app, &database, session)
        })
        .await;
    }

    session.begin_transaction().await?;
    let result = run_database_lifecycle_callback("seeder", &id, "run", || {
        seeder.run(&app, &database, session)
    })
    .await;
    match result {
        Ok(()) => session.commit_transaction().await,
        Err(error) => match session.rollback_transaction().await {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(Error::message(format!(
                "{error}; rollback failed: {rollback_error}"
            ))),
        },
    }
}

fn migration_run_in_transaction(migration: &dyn DynMigration, id: &MigrationId) -> Result<bool> {
    catch_sync_panic(|| migration.run_in_transaction()).map_err(|panic| {
        database_lifecycle_panic_error("migration", id, "run_in_transaction", panic)
    })
}

fn seeder_run_in_transaction(seeder: &dyn DynSeeder, id: &SeederId) -> Result<bool> {
    catch_sync_panic(|| seeder.run_in_transaction())
        .map_err(|panic| database_lifecycle_panic_error("seeder", id, "run_in_transaction", panic))
}

async fn run_database_lifecycle_callback<I, F, Fut>(
    kind: &'static str,
    id: &I,
    phase: &'static str,
    run: F,
) -> Result<()>
where
    I: std::fmt::Display,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    match catch_async_panic(run).await {
        Ok(result) => result,
        Err(panic) => Err(database_lifecycle_panic_error(kind, id, phase, panic)),
    }
}

fn database_lifecycle_panic_error<I>(
    kind: &'static str,
    id: &I,
    phase: &'static str,
    panic: Box<dyn std::any::Any + Send>,
) -> Error
where
    I: std::fmt::Display,
{
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.database.lifecycle",
        %kind,
        id = %id,
        %phase,
        panic = %message,
        "database lifecycle callback panicked"
    );
    Error::message(format!("{kind} `{id}` {phase} panicked: {message}"))
}

async fn ensure_ledger_table(config: &DatabaseConfig, executor: &dyn QueryExecutor) -> Result<()> {
    let schema = quote_identifier(&config.schema);
    executor
        .raw_execute(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"), &[])
        .await?;
    executor
        .raw_execute(
            &format!(
                "CREATE TABLE IF NOT EXISTS {} (id TEXT PRIMARY KEY, batch BIGINT NOT NULL, applied_at TIMESTAMPTZ NOT NULL)",
                qualified_migration_table(config)
            ),
            &[],
        )
        .await?;
    adopt_legacy_migration_ledger(config, executor).await?;
    Ok(())
}

async fn adopt_legacy_migration_ledger(
    config: &DatabaseConfig,
    executor: &dyn QueryExecutor,
) -> Result<()> {
    if config.migration_table != "foundry_migrations" {
        return Ok(());
    }

    executor
        .raw_execute(&legacy_migration_ledger_adoption_sql(config), &[])
        .await?;
    Ok(())
}

async fn applied_migrations(
    config: &DatabaseConfig,
    executor: &dyn QueryExecutor,
) -> Result<BTreeMap<MigrationId, AppliedMigration>> {
    let records = executor
        .raw_query(
            &format!(
                "SELECT id, batch, applied_at::TEXT AS applied_at FROM {} ORDER BY id",
                qualified_migration_table(config)
            ),
            &[],
        )
        .await?;

    let mut applied = BTreeMap::new();
    for record in records {
        let id = MigrationId::owned(record.decode::<String>("id")?);
        applied.insert(
            id.clone(),
            AppliedMigration {
                id,
                batch: record.decode("batch")?,
                applied_at: record.decode("applied_at")?,
            },
        );
    }
    Ok(applied)
}

fn ensure_applied_migrations_exist(
    applied: &BTreeMap<MigrationId, AppliedMigration>,
    migrations: &MigrationRegistry,
) -> Result<()> {
    let missing = missing_applied_migrations(applied, migrations);
    if let Some(migration) = missing.first() {
        return Err(Error::message(format!(
            "applied migration `{}` is missing from the registered migration set; run `db:migrate:status --json` to inspect migration drift. This usually means the current binary was built without a migration file that has already run, or the migration ledger points at a removed file.",
            migration.id
        )));
    }
    Ok(())
}

fn missing_applied_migrations(
    applied: &BTreeMap<MigrationId, AppliedMigration>,
    migrations: &MigrationRegistry,
) -> Vec<AppliedMigration> {
    applied
        .iter()
        .filter(|(id, _)| !migrations.contains(id))
        .map(|(_, migration)| migration.clone())
        .collect()
}

async fn record_applied_migration(
    executor: &dyn QueryExecutor,
    config: &DatabaseConfig,
    migration_id: &MigrationId,
    batch: i64,
) -> Result<()> {
    executor
        .raw_execute(
            &format!(
                "INSERT INTO {} (id, batch, applied_at) VALUES ($1, $2, NOW())",
                qualified_migration_table(config)
            ),
            &[migration_id.as_str().into(), batch.into()],
        )
        .await?;
    Ok(())
}

async fn delete_applied_migration(
    executor: &dyn QueryExecutor,
    config: &DatabaseConfig,
    migration_id: &MigrationId,
) -> Result<()> {
    executor
        .raw_execute(
            &format!(
                "DELETE FROM {} WHERE id = $1",
                qualified_migration_table(config)
            ),
            &[migration_id.as_str().into()],
        )
        .await?;
    Ok(())
}

fn qualified_migration_table(config: &DatabaseConfig) -> String {
    format!(
        "{}.{}",
        quote_identifier(&config.schema),
        quote_identifier(&config.migration_table)
    )
}

fn qualified_legacy_migration_table(config: &DatabaseConfig) -> String {
    format!(
        "{}.{}",
        quote_identifier(&config.schema),
        quote_identifier("forge_migrations")
    )
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn legacy_migration_ledger_adoption_sql(config: &DatabaseConfig) -> String {
    let legacy_table = qualified_legacy_migration_table(config);
    let migration_table = qualified_migration_table(config);
    let copy_sql = format!(
        "INSERT INTO {migration_table} (id, batch, applied_at) \
         SELECT id, batch, applied_at FROM {legacy_table} \
         ON CONFLICT (id) DO NOTHING"
    );

    format!(
        r#"
        DO $foundry$
        BEGIN
            IF to_regclass({legacy_table_literal}) IS NOT NULL THEN
                EXECUTE {copy_sql_literal};
            END IF;
        END
        $foundry$;
        "#,
        legacy_table_literal = quote_literal(&legacy_table),
        copy_sql_literal = quote_literal(&copy_sql),
    )
}

fn advisory_lock_key(config: &DatabaseConfig) -> i64 {
    let input = format!("foundry:{}:{}", config.schema, config.migration_table);
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in input.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    (hash & 0x7fff_ffff_ffff_ffff) as i64
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::{
        acquire_migration_lock, advisory_lock_key, legacy_migration_ledger_adoption_sql,
        migration_lock_timeout_error, migration_run_in_transaction, missing_applied_migrations,
        run_database_lifecycle_callback, seeder_run_in_transaction, AppliedMigration,
        GeneratedDatabasePaths, MigrationContext, MigrationFile, MigrationFileAdapter, MigrationId,
        MigrationLockClient, MigrationRegistryBuilder, MigrationStatus, MigrationStatusReport,
        SeederContext, SeederFile, SeederFileAdapter, SeederId, SeederRegistryBuilder,
    };
    use crate::config::DatabaseConfig;
    use crate::foundation::{Error, Result};

    struct CreateUsers;

    #[async_trait]
    impl MigrationFile for CreateUsers {
        async fn up(_ctx: &MigrationContext<'_>) -> Result<()> {
            Ok(())
        }

        async fn down(_ctx: &MigrationContext<'_>) -> Result<()> {
            Ok(())
        }
    }

    struct SeedUsers;

    #[async_trait]
    impl SeederFile for SeedUsers {
        async fn run(_ctx: &SeederContext<'_>) -> Result<()> {
            Ok(())
        }
    }

    struct FileSeedUsers;

    #[async_trait]
    impl SeederFile for FileSeedUsers {
        async fn run(_ctx: &SeederContext<'_>) -> Result<()> {
            Ok(())
        }
    }

    struct PanickingTransactionMigration;

    #[async_trait]
    impl MigrationFile for PanickingTransactionMigration {
        fn run_in_transaction() -> bool {
            panic!("migration transaction flag exploded")
        }

        async fn up(_ctx: &MigrationContext<'_>) -> Result<()> {
            Ok(())
        }

        async fn down(_ctx: &MigrationContext<'_>) -> Result<()> {
            Ok(())
        }
    }

    struct PanickingTransactionSeeder;

    struct FakeMigrationLockClient {
        attempts_before_success: usize,
        attempts: AtomicUsize,
    }

    #[async_trait]
    impl MigrationLockClient for FakeMigrationLockClient {
        async fn try_acquire_migration_lock(&self, _lock_key: i64) -> Result<bool> {
            let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
            Ok(attempt >= self.attempts_before_success)
        }
    }

    #[async_trait]
    impl SeederFile for PanickingTransactionSeeder {
        fn run_in_transaction() -> bool {
            panic!("seeder transaction flag exploded")
        }

        async fn run(_ctx: &SeederContext<'_>) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn rejects_duplicate_migration_ids() {
        let mut builder = MigrationRegistryBuilder::default();
        builder
            .register_file::<CreateUsers>(MigrationId::new("202604091200_create_users"))
            .unwrap();

        let error = builder
            .register_file::<CreateUsers>(MigrationId::new("202604091200_create_users"))
            .err()
            .unwrap();
        assert!(error.to_string().contains("already registered"));
    }

    #[test]
    fn rejects_duplicate_seeder_ids() {
        let mut builder = SeederRegistryBuilder::default();
        builder
            .register_file::<SeedUsers>(SeederId::new("users.seed"))
            .unwrap();

        let error = builder
            .register_file::<SeedUsers>(SeederId::new("users.seed"))
            .err()
            .unwrap();
        assert!(error.to_string().contains("already registered"));
    }

    #[test]
    fn preserves_seeder_registration_order() {
        let mut builder = SeederRegistryBuilder::default();
        builder
            .register_file::<SeedUsers>(SeederId::new("users.seed"))
            .unwrap();
        builder
            .register_file::<FileSeedUsers>(SeederId::new("users.file"))
            .unwrap();

        let registry = SeederRegistryBuilder::freeze_shared(Arc::new(Mutex::new(builder))).unwrap();
        assert_eq!(
            registry.ids(),
            vec![SeederId::new("users.seed"), SeederId::new("users.file")]
        );
    }

    #[tokio::test]
    async fn migration_callback_future_panic_becomes_error() {
        let id = MigrationId::new("202604101532_panic_migration");
        let error = run_database_lifecycle_callback("migration", &id, "up", || async {
            let should_panic = true;
            if should_panic {
                panic!("migration up exploded");
            }
            Ok::<(), Error>(())
        })
        .await
        .expect_err("migration panic should become an error");

        assert_eq!(
            error.to_string(),
            "migration `202604101532_panic_migration` up panicked: migration up exploded"
        );
    }

    #[tokio::test]
    async fn seeder_callback_factory_panic_becomes_error() {
        let id = SeederId::new("panic_seed");
        let error = run_database_lifecycle_callback("seeder", &id, "run", || {
            panic!("seeder future factory exploded");
            #[allow(unreachable_code)]
            std::future::ready(Ok::<(), Error>(()))
        })
        .await
        .expect_err("seeder factory panic should become an error");

        assert_eq!(
            error.to_string(),
            "seeder `panic_seed` run panicked: seeder future factory exploded"
        );
    }

    #[tokio::test]
    async fn lifecycle_callback_error_remains_unchanged() {
        let id = SeederId::new("error_seed");
        let error = run_database_lifecycle_callback("seeder", &id, "run", || async {
            Err(Error::message("seed returned error"))
        })
        .await
        .expect_err("normal errors should remain unchanged");

        assert_eq!(error.to_string(), "seed returned error");
    }

    #[test]
    fn migration_run_in_transaction_panic_becomes_error() {
        let id = MigrationId::new("202604101533_panic_transaction_flag");
        let migration = MigrationFileAdapter::<PanickingTransactionMigration> {
            id: id.clone(),
            marker: std::marker::PhantomData,
        };
        let error = migration_run_in_transaction(&migration, &id)
            .expect_err("migration transaction flag panic should become an error");

        assert_eq!(
            error.to_string(),
            "migration `202604101533_panic_transaction_flag` run_in_transaction panicked: migration transaction flag exploded"
        );
    }

    #[test]
    fn seeder_run_in_transaction_panic_becomes_error() {
        let id = SeederId::new("panic_transaction_seed");
        let seeder = SeederFileAdapter::<PanickingTransactionSeeder> {
            id: id.clone(),
            marker: std::marker::PhantomData,
        };
        let error = seeder_run_in_transaction(&seeder, &id)
            .expect_err("seeder transaction flag panic should become an error");

        assert_eq!(
            error.to_string(),
            "seeder `panic_transaction_seed` run_in_transaction panicked: seeder transaction flag exploded"
        );
    }

    #[test]
    fn maps_applied_and_pending_statuses() {
        let applied = AppliedMigration {
            id: MigrationId::new("202604090900_init"),
            batch: 1,
            applied_at: "2026-04-09 09:00:00+00".to_string(),
        };
        let statuses = [
            MigrationStatus {
                id: applied.id.clone(),
                applied: Some(applied.clone()),
            },
            MigrationStatus {
                id: MigrationId::new("202604091000_users"),
                applied: None,
            },
        ];

        assert_eq!(statuses[0].applied.as_ref(), Some(&applied));
        assert!(statuses[1].applied.is_none());
    }

    #[test]
    fn migration_status_json_includes_summary_and_missing_applied_rows() {
        let applied = AppliedMigration {
            id: MigrationId::new("202604090900_init"),
            batch: 1,
            applied_at: "2026-04-09 09:00:00+00".to_string(),
        };
        let missing = AppliedMigration {
            id: MigrationId::new("202604090800_removed"),
            batch: 1,
            applied_at: "2026-04-09 08:00:00+00".to_string(),
        };
        let report = MigrationStatusReport {
            statuses: vec![
                MigrationStatus {
                    id: applied.id.clone(),
                    applied: Some(applied),
                },
                MigrationStatus {
                    id: MigrationId::new("202604091000_users"),
                    applied: None,
                },
            ],
            missing_applied: vec![missing],
            latest_batch: Some(1),
        };

        let json = serde_json::to_value(report.to_json()).unwrap();
        assert_eq!(json["summary"]["registered"], 2);
        assert_eq!(json["summary"]["applied"], 1);
        assert_eq!(json["summary"]["pending"], 1);
        assert_eq!(json["summary"]["missing_applied"], 1);
        assert_eq!(json["summary"]["drifted"], true);
        assert_eq!(json["summary"]["up_to_date"], false);
        assert_eq!(json["migrations"][0]["state"], "applied");
        assert_eq!(json["migrations"][1]["state"], "pending");
        assert_eq!(json["missing_applied"][0]["state"], "missing_applied");
    }

    #[test]
    fn migration_status_json_marks_clean_fully_applied_reports_up_to_date() {
        let applied = AppliedMigration {
            id: MigrationId::new("202604090900_init"),
            batch: 1,
            applied_at: "2026-04-09 09:00:00+00".to_string(),
        };
        let report = MigrationStatusReport {
            statuses: vec![MigrationStatus {
                id: applied.id.clone(),
                applied: Some(applied),
            }],
            missing_applied: Vec::new(),
            latest_batch: Some(1),
        };

        let json = serde_json::to_value(report.to_json()).unwrap();
        assert_eq!(json["summary"]["registered"], 1);
        assert_eq!(json["summary"]["applied"], 1);
        assert_eq!(json["summary"]["pending"], 0);
        assert_eq!(json["summary"]["missing_applied"], 0);
        assert_eq!(json["summary"]["drifted"], false);
        assert_eq!(json["summary"]["up_to_date"], true);
    }

    #[test]
    fn detects_missing_applied_migrations() {
        let mut builder = MigrationRegistryBuilder::default();
        builder
            .register_file::<CreateUsers>(MigrationId::new("202604091200_create_users"))
            .unwrap();
        let registry =
            MigrationRegistryBuilder::freeze_shared(Arc::new(Mutex::new(builder))).unwrap();
        let mut applied = std::collections::BTreeMap::new();
        applied.insert(
            MigrationId::new("202604091200_create_users"),
            AppliedMigration {
                id: MigrationId::new("202604091200_create_users"),
                batch: 1,
                applied_at: "2026-04-09 12:00:00+00".to_string(),
            },
        );
        applied.insert(
            MigrationId::new("202604091300_removed"),
            AppliedMigration {
                id: MigrationId::new("202604091300_removed"),
                batch: 1,
                applied_at: "2026-04-09 13:00:00+00".to_string(),
            },
        );

        let missing = missing_applied_migrations(&applied, &registry);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].id, MigrationId::new("202604091300_removed"));
    }

    #[tokio::test]
    async fn migration_lock_wait_loop_succeeds_after_retry() {
        let client = FakeMigrationLockClient {
            attempts_before_success: 2,
            attempts: AtomicUsize::new(0),
        };
        acquire_migration_lock(&client, &DatabaseConfig::default(), 42, 1_000)
            .await
            .unwrap();
        assert_eq!(client.attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn migration_lock_wait_loop_times_out() {
        let client = FakeMigrationLockClient {
            attempts_before_success: usize::MAX,
            attempts: AtomicUsize::new(0),
        };
        let config = DatabaseConfig {
            schema: "foundry_ops".to_string(),
            migration_table: "schema_migrations".to_string(),
            ..DatabaseConfig::default()
        };
        let error = acquire_migration_lock(&client, &config, 42, 1)
            .await
            .expect_err("lock wait should time out");

        assert_eq!(
            error.to_string(),
            "timed out after 1ms waiting for migration advisory lock for schema `foundry_ops` and migration table `schema_migrations`; another db:migrate or db:rollback process is likely still running"
        );
    }

    #[test]
    fn migration_lock_timeout_error_names_schema_and_table() {
        let config = DatabaseConfig {
            schema: "foundry_ops".to_string(),
            migration_table: "schema_migrations".to_string(),
            ..DatabaseConfig::default()
        };
        let error = migration_lock_timeout_error(&config, 250);
        assert!(error.to_string().contains("foundry_ops"));
        assert!(error.to_string().contains("schema_migrations"));
        assert!(error.to_string().contains("250ms"));
    }

    #[test]
    fn generated_database_paths_expose_primary_dirs() {
        let paths = GeneratedDatabasePaths::new(
            vec!["/tmp/migrations".into()],
            vec!["/tmp/seeders".into()],
        );
        assert_eq!(
            paths.primary_migration_dir().unwrap(),
            std::path::Path::new("/tmp/migrations")
        );
        assert_eq!(
            paths.primary_seeder_dir().unwrap(),
            std::path::Path::new("/tmp/seeders")
        );
    }

    #[test]
    fn advisory_lock_key_depends_on_schema_and_table() {
        let public = DatabaseConfig::default();
        let custom = DatabaseConfig {
            schema: "foundry".to_string(),
            migration_table: "custom_migrations".to_string(),
            ..DatabaseConfig::default()
        };

        assert_ne!(advisory_lock_key(&public), advisory_lock_key(&custom));
    }

    #[test]
    fn legacy_migration_ledger_adoption_copies_forge_rows_into_foundry_table() {
        let config = DatabaseConfig {
            schema: "public".to_string(),
            migration_table: "foundry_migrations".to_string(),
            ..DatabaseConfig::default()
        };

        let sql = legacy_migration_ledger_adoption_sql(&config);

        assert!(sql.contains("to_regclass('\"public\".\"forge_migrations\"')"));
        assert!(sql.contains("INSERT INTO \"public\".\"foundry_migrations\""));
        assert!(sql.contains("FROM \"public\".\"forge_migrations\""));
        assert!(sql.contains("ON CONFLICT (id) DO NOTHING"));
    }
}
