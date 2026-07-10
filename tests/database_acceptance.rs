use std::fs;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use foundry::config::DatabaseConfig;
use foundry::prelude::*;
use futures_util::TryStreamExt;
use tempfile::TempDir;
use tokio::sync::Mutex;
use uuid::Uuid;

const USERS_TABLE: &str = "foundry_test_users";
const MERCHANTS_TABLE: &str = "foundry_test_merchants";
const USER_PROFILES_TABLE: &str = "foundry_test_user_profiles";
const ORDERS_TABLE: &str = "foundry_test_orders";
const ORDER_ITEMS_TABLE: &str = "foundry_test_order_items";
const PRODUCTS_TABLE: &str = "foundry_test_products";
const TAGS_TABLE: &str = "foundry_test_tags";
const MERCHANT_TAGS_TABLE: &str = "foundry_test_merchant_tags";
const PAYMENTS_TABLE: &str = "foundry_test_payments";
const POSTS_TABLE: &str = "foundry_test_posts";
const NUMERIC_POSTS_TABLE: &str = "foundry_test_numeric_posts";
const SAFE_USERS_TABLE: &str = "foundry_test_safe_uuid_users";
const PASSWORD_USERS_TABLE: &str = "foundry_test_password_users";
const COUNTRIES_RUNTIME_TABLE: &str = "foundry_test_runtime_countries";

fn database_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn postgres_url() -> Option<String> {
    std::env::var("FOUNDRY_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

async fn test_database() -> Option<DatabaseManager> {
    let url = postgres_url()?;
    Some(
        DatabaseManager::from_config(&DatabaseConfig {
            url,
            ..DatabaseConfig::default()
        })
        .await
        .unwrap(),
    )
}

#[tokio::test]
async fn failed_session_query_resets_statement_timeout_before_pool_reuse() {
    let Some(url) = postgres_url() else {
        return;
    };
    let database = DatabaseManager::from_config(&DatabaseConfig {
        url,
        max_connections: 1,
        ..DatabaseConfig::default()
    })
    .await
    .unwrap();

    database
        .raw_query_with(
            "SELECT * FROM foundry_table_that_does_not_exist",
            &[],
            QueryExecutionOptions::default().with_timeout(Duration::from_millis(1234)),
        )
        .await
        .unwrap_err();

    let statement_timeout: String = sqlx::query_scalar("SHOW statement_timeout")
        .fetch_one(database.pool().unwrap())
        .await
        .unwrap();

    assert_eq!(statement_timeout, "0");
}

struct TestAppRuntime {
    _dir: TempDir,
    app: AppContext,
    database: std::sync::Arc<DatabaseManager>,
}

#[derive(Clone)]
struct NoopProvider;

#[async_trait]
impl ServiceProvider for NoopProvider {}

async fn test_app_runtime() -> Option<TestAppRuntime> {
    test_app_runtime_with_provider(NoopProvider).await
}

async fn test_app_runtime_with_provider<P>(provider: P) -> Option<TestAppRuntime>
where
    P: ServiceProvider,
{
    test_app_runtime_with_provider_and_config(provider, "").await
}

async fn test_app_runtime_with_provider_and_config<P>(
    provider: P,
    extra_config: &str,
) -> Option<TestAppRuntime>
where
    P: ServiceProvider,
{
    let url = postgres_url()?;
    let dir = tempfile::tempdir().ok()?;
    fs::write(
        dir.path().join("00-runtime.toml"),
        format!(
            r#"
            [database]
            url = "{url}"
            {extra_config}
            "#
        ),
    )
    .ok()?;

    let kernel = App::builder()
        .load_config_dir(dir.path())
        .register_provider(provider)
        .build_cli_kernel()
        .await
        .ok()?;
    let app = kernel.app().clone();
    let database = app.database().ok()?;

    Some(TestAppRuntime {
        _dir: dir,
        app,
        database,
    })
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
            &format!("DROP TABLE IF EXISTS {ORDER_ITEMS_TABLE}"),
            &format!("DROP TABLE IF EXISTS {MERCHANT_TAGS_TABLE}"),
            &format!("DROP TABLE IF EXISTS {ORDERS_TABLE}"),
            &format!("DROP TABLE IF EXISTS {TAGS_TABLE}"),
            &format!("DROP TABLE IF EXISTS {PAYMENTS_TABLE}"),
            &format!("DROP TABLE IF EXISTS {POSTS_TABLE}"),
            &format!("DROP TABLE IF EXISTS {NUMERIC_POSTS_TABLE}"),
            &format!("DROP TABLE IF EXISTS {PASSWORD_USERS_TABLE}"),
            &format!("DROP TABLE IF EXISTS {USER_PROFILES_TABLE}"),
            &format!("DROP TABLE IF EXISTS {MERCHANTS_TABLE}"),
            &format!("DROP TABLE IF EXISTS {PRODUCTS_TABLE}"),
            &format!("DROP TABLE IF EXISTS {USERS_TABLE}"),
            &format!(
                "CREATE TABLE {USERS_TABLE} (id BIGINT PRIMARY KEY, email TEXT NOT NULL, active BOOLEAN NOT NULL, nickname TEXT NULL, metadata JSONB NOT NULL DEFAULT '{{}}'::jsonb)"
            ),
            &format!(
                "CREATE TABLE {MERCHANTS_TABLE} (id BIGINT PRIMARY KEY, user_id BIGINT NOT NULL, name TEXT NOT NULL, status TEXT NOT NULL)"
            ),
            &format!(
                "CREATE TABLE {USER_PROFILES_TABLE} (id BIGINT PRIMARY KEY, user_id BIGINT NOT NULL REFERENCES {USERS_TABLE}(id), label TEXT NOT NULL)"
            ),
            &format!(
                "CREATE TABLE {ORDERS_TABLE} (id BIGINT PRIMARY KEY, merchant_id BIGINT NOT NULL, total BIGINT NOT NULL)"
            ),
            &format!(
                "CREATE TABLE {PRODUCTS_TABLE} (id BIGINT PRIMARY KEY, name TEXT NOT NULL)"
            ),
            &format!(
                "CREATE TABLE {TAGS_TABLE} (id BIGINT PRIMARY KEY, user_id BIGINT NOT NULL, name TEXT NOT NULL)"
            ),
            &format!(
                "CREATE TABLE {MERCHANT_TAGS_TABLE} (merchant_id BIGINT NOT NULL, tag_id BIGINT NOT NULL, role TEXT NOT NULL)"
            ),
            &format!(
                "CREATE TABLE {PAYMENTS_TABLE} (id BIGINT PRIMARY KEY, merchant_id BIGINT NOT NULL, amount NUMERIC NOT NULL, metadata JSONB NOT NULL DEFAULT '{{}}'::jsonb)"
            ),
            &format!(
                "CREATE TABLE {POSTS_TABLE} (id BIGINT PRIMARY KEY, title TEXT NOT NULL, body TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL, updated_at TIMESTAMPTZ NOT NULL, deleted_at TIMESTAMPTZ NULL)"
            ),
            &format!(
                "CREATE TABLE {NUMERIC_POSTS_TABLE} (id BIGINT PRIMARY KEY, amount NUMERIC(20,8) NOT NULL, note TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL, updated_at TIMESTAMPTZ NOT NULL, deleted_at TIMESTAMPTZ NULL)"
            ),
            &format!(
                "CREATE TABLE {PASSWORD_USERS_TABLE} (id BIGINT PRIMARY KEY, email TEXT NOT NULL, password TEXT NOT NULL)"
            ),
            &format!(
                "CREATE TABLE {ORDER_ITEMS_TABLE} (id BIGINT PRIMARY KEY, order_id BIGINT NOT NULL, product_id BIGINT NOT NULL, quantity BIGINT NOT NULL)"
            ),
        ],
    )
    .await;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MerchantStatus {
    Active,
    Suspended,
}

impl ToDbValue for MerchantStatus {
    fn to_db_value(self) -> DbValue {
        match self {
            Self::Active => "active".into(),
            Self::Suspended => "suspended".into(),
        }
    }

    fn db_type() -> DbType {
        DbType::Text
    }
}

impl FromDbValue for MerchantStatus {
    fn from_db_value(value: &DbValue) -> Result<Self> {
        match value {
            DbValue::Text(value) if value == "active" => Ok(Self::Active),
            DbValue::Text(value) if value == "suspended" => Ok(Self::Suspended),
            DbValue::Text(_) => Err(Error::message("unknown merchant status")),
            DbValue::Null(_) => Err(Error::message("expected merchant status, found null")),
            _ => Err(Error::message("expected merchant status text value")),
        }
    }
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = USERS_TABLE, primary_key_strategy = "manual")]
struct User {
    id: i64,
    email: String,
    active: bool,
    #[foundry(write_mutator = "normalize_nickname")]
    nickname: Option<String>,
    merchants: Loaded<Vec<Merchant>>,
    merchant_count: Loaded<i64>,
}

impl User {
    async fn normalize_nickname(
        _context: &ModelHookContext<'_>,
        value: Option<String>,
    ) -> Result<Option<String>> {
        Ok(value.map(|nickname| nickname.trim().to_lowercase()))
    }
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = MERCHANTS_TABLE, primary_key_strategy = "manual")]
struct Merchant {
    id: i64,
    user_id: i64,
    name: String,
    #[foundry(db_type = "text")]
    status: MerchantStatus,
    orders: Loaded<Vec<Order>>,
    order_total: Loaded<Option<i64>>,
    tags: Loaded<Vec<Tag>>,
    tag_count: Loaded<i64>,
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = ORDERS_TABLE, primary_key_strategy = "manual")]
struct Order {
    id: i64,
    merchant_id: i64,
    total: i64,
    items: Loaded<Vec<OrderItem>>,
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = ORDER_ITEMS_TABLE, primary_key_strategy = "manual")]
struct OrderItem {
    id: i64,
    order_id: i64,
    product_id: i64,
    quantity: i64,
    product: Loaded<Option<Product>>,
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = PRODUCTS_TABLE, primary_key_strategy = "manual")]
struct Product {
    id: i64,
    name: String,
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = TAGS_TABLE, primary_key_strategy = "manual")]
struct Tag {
    id: i64,
    user_id: i64,
    name: String,
    link: Loaded<MerchantTagLink>,
    creator: Loaded<Option<TagCreator>>,
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = USERS_TABLE, primary_key_strategy = "manual")]
struct TagCreator {
    id: i64,
    email: String,
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = POSTS_TABLE, primary_key_strategy = "manual", soft_deletes = true)]
struct PostRecord {
    id: i64,
    title: String,
    body: String,
    created_at: DateTime,
    updated_at: DateTime,
    deleted_at: Option<DateTime>,
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = NUMERIC_POSTS_TABLE, primary_key_strategy = "manual", soft_deletes = true)]
struct NumericPostRecord {
    id: i64,
    amount: Numeric,
    note: String,
    created_at: DateTime,
    updated_at: DateTime,
    deleted_at: Option<DateTime>,
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = POSTS_TABLE, primary_key_strategy = "manual")]
struct DefaultSoftDeletePost {
    id: i64,
    title: String,
    body: String,
    created_at: DateTime,
    updated_at: DateTime,
    deleted_at: Option<DateTime>,
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = POSTS_TABLE, primary_key_strategy = "manual", soft_deletes = false)]
struct OptOutSoftDeletePost {
    id: i64,
    title: String,
    body: String,
    created_at: DateTime,
    updated_at: DateTime,
    deleted_at: Option<DateTime>,
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = PASSWORD_USERS_TABLE, primary_key_strategy = "manual")]
struct PasswordUser {
    id: i64,
    email: String,
    #[foundry(write_mutator = "hash_password")]
    #[foundry(read_accessor = "masked_password")]
    password: String,
}

impl PasswordUser {
    async fn hash_password(context: &ModelHookContext<'_>, value: String) -> Result<String> {
        context.app().hash()?.hash(&value)
    }

    fn masked_password(&self) -> String {
        "********".to_string()
    }
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = PASSWORD_USERS_TABLE, primary_key_strategy = "manual")]
struct PanickingPasswordUser {
    id: i64,
    email: String,
    #[foundry(write_mutator = "panic_password")]
    password: String,
}

impl PanickingPasswordUser {
    async fn panic_password(_context: &ModelHookContext<'_>, value: String) -> Result<String> {
        if value == "panic-create" || value == "panic-update" {
            panic!("password mutator boom");
        }
        Ok(format!("mutated:{value}"))
    }
}

#[derive(Default)]
struct TimestampHookLog {
    entries: Mutex<Vec<String>>,
}

impl TimestampHookLog {
    async fn push(&self, entry: impl Into<String>) {
        self.entries.lock().await.push(entry.into());
    }

    async fn snapshot(&self) -> Vec<String> {
        self.entries.lock().await.clone()
    }

    async fn clear(&self) {
        self.entries.lock().await.clear();
    }
}

#[derive(Clone)]
struct TimestampHookProvider {
    log: std::sync::Arc<TimestampHookLog>,
}

#[async_trait]
impl ServiceProvider for TimestampHookProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.singleton_arc(self.log.clone())
    }
}

struct HookedPostLifecycle;

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = POSTS_TABLE, primary_key_strategy = "manual", soft_deletes = true, lifecycle = HookedPostLifecycle)]
struct HookedPost {
    id: i64,
    title: String,
    body: String,
    created_at: DateTime,
    updated_at: DateTime,
    deleted_at: Option<DateTime>,
}

fn fixed_time(day: u32) -> DateTime {
    DateTime::parse(format!("2026-01-{day:02}T10:00:00Z")).unwrap()
}

#[test]
fn nullable_column_compare_ops_accept_inner_values() {
    let now = fixed_time(10);

    let _ = OptOutSoftDeletePost::DELETED_AT.eq(now);
    let _ = OptOutSoftDeletePost::DELETED_AT.not_eq(now);
    let _ = OptOutSoftDeletePost::DELETED_AT.lt(now);
    let _ = OptOutSoftDeletePost::DELETED_AT.lte(now);
    let _ = OptOutSoftDeletePost::DELETED_AT.gt(now);
    let _ = OptOutSoftDeletePost::DELETED_AT.gte(now);
    let _ = OptOutSoftDeletePost::DELETED_AT.in_list([now]);

    assert_eq!(
        None::<DateTime>.to_db_value(),
        DbValue::Null(DbType::TimestampTz)
    );

    assert!(matches!(
        OptOutSoftDeletePost::DELETED_AT.eq(None::<DateTime>),
        Condition::Comparison {
            right: Expr::Value(DbValue::Null(DbType::TimestampTz)),
            ..
        }
    ));
}

#[async_trait]
impl ModelLifecycle<HookedPost> for HookedPostLifecycle {
    async fn creating(
        context: &ModelHookContext<'_>,
        draft: &mut CreateDraft<HookedPost>,
    ) -> Result<()> {
        let pending = draft.pending_record();
        context
            .app()
            .resolve::<TimestampHookLog>()?
            .push(format!(
                "creating:{}:{}",
                pending.get("created_at").is_some(),
                pending.get("updated_at").is_some()
            ))
            .await;
        draft.set(HookedPost::UPDATED_AT, fixed_time(2));
        Ok(())
    }

    async fn updating(
        context: &ModelHookContext<'_>,
        _current: &HookedPost,
        draft: &mut UpdateDraft<HookedPost>,
    ) -> Result<()> {
        let pending = draft.pending_record();
        context
            .app()
            .resolve::<TimestampHookLog>()?
            .push(format!("updating:{}", pending.get("updated_at").is_some()))
            .await;
        draft.set(HookedPost::UPDATED_AT, fixed_time(3));
        Ok(())
    }

    async fn deleting(
        context: &ModelHookContext<'_>,
        _current: &HookedPost,
        _record: &DbRecord,
    ) -> Result<()> {
        context
            .app()
            .resolve::<TimestampHookLog>()?
            .push("deleting")
            .await;
        Ok(())
    }

    async fn deleted(
        context: &ModelHookContext<'_>,
        _deleted: &HookedPost,
        _record: &DbRecord,
    ) -> Result<()> {
        context
            .app()
            .resolve::<TimestampHookLog>()?
            .push("deleted")
            .await;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, foundry::Projection)]
struct MerchantTagLink {
    #[foundry(source = "role")]
    role: String,
}

#[derive(Clone, Debug, PartialEq, foundry::Projection)]
struct UserPreferenceRow {
    #[foundry(source = "email")]
    email: String,
    #[foundry(source = "status_label")]
    status_label: String,
    #[foundry(source = "theme")]
    theme: String,
}

#[derive(Clone, Debug, PartialEq, foundry::Projection)]
struct CombinedLabelRow {
    label: String,
    kind: String,
}

#[derive(Default)]
struct LifecycleLog {
    entries: Mutex<Vec<String>>,
}

impl LifecycleLog {
    async fn push(&self, entry: impl Into<String>) {
        self.entries.lock().await.push(entry.into());
    }

    async fn snapshot(&self) -> Vec<String> {
        self.entries.lock().await.clone()
    }

    async fn clear(&self) {
        self.entries.lock().await.clear();
    }
}

#[derive(Clone, serde::Serialize)]
struct LifecycleCustomEvent {
    phase: String,
}

impl Event for LifecycleCustomEvent {
    const ID: EventId = EventId::new("test.lifecycle.custom");
}

#[derive(Clone)]
struct LifecycleTestProvider {
    log: std::sync::Arc<LifecycleLog>,
    fail_created: bool,
}

struct LifecycleEventListener<E> {
    log: std::sync::Arc<LifecycleLog>,
    label: &'static str,
    marker: std::marker::PhantomData<E>,
}

impl<E> LifecycleEventListener<E> {
    fn new(log: std::sync::Arc<LifecycleLog>, label: &'static str) -> Self {
        Self {
            log,
            label,
            marker: std::marker::PhantomData,
        }
    }
}

macro_rules! impl_lifecycle_event_listener {
    ($event:ty) => {
        #[async_trait]
        impl EventListener<$event> for LifecycleEventListener<$event> {
            async fn handle(&self, _context: &EventContext, event: &$event) -> Result<()> {
                if event.snapshot.table == USERS_TABLE {
                    self.log.push(format!("event:{}", self.label)).await;
                }
                Ok(())
            }
        }
    };
}

impl_lifecycle_event_listener!(ModelCreatingEvent);
impl_lifecycle_event_listener!(ModelCreatedEvent);
impl_lifecycle_event_listener!(ModelUpdatingEvent);
impl_lifecycle_event_listener!(ModelUpdatedEvent);
impl_lifecycle_event_listener!(ModelDeletingEvent);
impl_lifecycle_event_listener!(ModelDeletedEvent);

struct LifecycleCustomEventListener {
    log: std::sync::Arc<LifecycleLog>,
}

#[async_trait]
impl EventListener<LifecycleCustomEvent> for LifecycleCustomEventListener {
    async fn handle(&self, _context: &EventContext, event: &LifecycleCustomEvent) -> Result<()> {
        self.log.push(format!("custom:{}", event.phase)).await;
        Ok(())
    }
}

struct FailingCreatedEventListener;

#[async_trait]
impl EventListener<ModelCreatedEvent> for FailingCreatedEventListener {
    async fn handle(&self, _context: &EventContext, event: &ModelCreatedEvent) -> Result<()> {
        if event.snapshot.table == USERS_TABLE {
            return Err(Error::message("created event failed"));
        }
        Ok(())
    }
}

struct ProfileOnUserCreatedListener;

#[async_trait]
impl EventListener<ModelCreatedEvent> for ProfileOnUserCreatedListener {
    async fn handle(&self, ctx: &EventContext, event: &ModelCreatedEvent) -> Result<()> {
        if event.snapshot.table != USERS_TABLE {
            return Ok(());
        }

        let Some(after) = event.snapshot.after.as_ref() else {
            return Ok(());
        };
        let user_id = after.decode::<i64>("id")?;

        Query::insert_into(USER_PROFILES_TABLE)
            .value("id", user_id + 10_000)
            .value("user_id", user_id)
            .value("label", "created-after-commit")
            .execute(ctx.app())
            .await?;

        Ok(())
    }
}

#[derive(Clone)]
struct ProfileOnUserCreatedProvider;

#[async_trait]
impl ServiceProvider for ProfileOnUserCreatedProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.listen_event::<ModelCreatedEvent, _>(ProfileOnUserCreatedListener)?;
        Ok(())
    }
}

#[async_trait]
impl ServiceProvider for LifecycleTestProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.singleton_arc(self.log.clone())?;
        registrar.listen_event::<ModelCreatingEvent, _>(LifecycleEventListener::new(
            self.log.clone(),
            "creating",
        ))?;
        registrar.listen_event::<ModelCreatedEvent, _>(LifecycleEventListener::new(
            self.log.clone(),
            "created",
        ))?;
        registrar.listen_event::<ModelUpdatingEvent, _>(LifecycleEventListener::new(
            self.log.clone(),
            "updating",
        ))?;
        registrar.listen_event::<ModelUpdatedEvent, _>(LifecycleEventListener::new(
            self.log.clone(),
            "updated",
        ))?;
        registrar.listen_event::<ModelDeletingEvent, _>(LifecycleEventListener::new(
            self.log.clone(),
            "deleting",
        ))?;
        registrar.listen_event::<ModelDeletedEvent, _>(LifecycleEventListener::new(
            self.log.clone(),
            "deleted",
        ))?;
        registrar.listen_event::<LifecycleCustomEvent, _>(LifecycleCustomEventListener {
            log: self.log.clone(),
        })?;
        if self.fail_created {
            registrar.listen_event::<ModelCreatedEvent, _>(FailingCreatedEventListener)?;
        }
        Ok(())
    }
}

struct LifecycleUserHooks;

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = USERS_TABLE, primary_key_strategy = "manual", lifecycle = LifecycleUserHooks)]
struct LifecycleUser {
    id: i64,
    email: String,
    active: bool,
    nickname: Option<String>,
}

#[async_trait]
impl ModelLifecycle<LifecycleUser> for LifecycleUserHooks {
    async fn creating(
        context: &ModelHookContext<'_>,
        draft: &mut CreateDraft<LifecycleUser>,
    ) -> Result<()> {
        context
            .app()
            .resolve::<LifecycleLog>()?
            .push("hook:creating")
            .await;
        draft.set(LifecycleUser::NICKNAME, "hook-created");
        context
            .dispatch(LifecycleCustomEvent {
                phase: "creating".to_string(),
            })
            .await?;
        Ok(())
    }

    async fn created(
        context: &ModelHookContext<'_>,
        _created: &LifecycleUser,
        _record: &DbRecord,
    ) -> Result<()> {
        context
            .app()
            .resolve::<LifecycleLog>()?
            .push("hook:created")
            .await;
        Ok(())
    }

    async fn updating(
        context: &ModelHookContext<'_>,
        _current: &LifecycleUser,
        draft: &mut UpdateDraft<LifecycleUser>,
    ) -> Result<()> {
        context
            .app()
            .resolve::<LifecycleLog>()?
            .push("hook:updating")
            .await;
        if draft.pending_record().get("nickname").is_none() {
            draft.set(LifecycleUser::NICKNAME, "hook-updated");
        }
        context
            .dispatch(LifecycleCustomEvent {
                phase: "updating".to_string(),
            })
            .await?;
        Ok(())
    }

    async fn updated(
        context: &ModelHookContext<'_>,
        _before: &LifecycleUser,
        _after: &LifecycleUser,
        _before_record: &DbRecord,
        _after_record: &DbRecord,
    ) -> Result<()> {
        context
            .app()
            .resolve::<LifecycleLog>()?
            .push("hook:updated")
            .await;
        Ok(())
    }

    async fn deleting(
        context: &ModelHookContext<'_>,
        _current: &LifecycleUser,
        _record: &DbRecord,
    ) -> Result<()> {
        context
            .app()
            .resolve::<LifecycleLog>()?
            .push("hook:deleting")
            .await;
        Ok(())
    }

    async fn deleted(
        context: &ModelHookContext<'_>,
        _deleted: &LifecycleUser,
        _record: &DbRecord,
    ) -> Result<()> {
        context
            .app()
            .resolve::<LifecycleLog>()?
            .push("hook:deleted")
            .await;
        Ok(())
    }
}

struct RejectingLifecycleUserHooks;

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = USERS_TABLE, primary_key_strategy = "manual", lifecycle = RejectingLifecycleUserHooks)]
struct RejectingLifecycleUser {
    id: i64,
    email: String,
    active: bool,
    nickname: Option<String>,
}

#[async_trait]
impl ModelLifecycle<RejectingLifecycleUser> for RejectingLifecycleUserHooks {
    async fn creating(
        _context: &ModelHookContext<'_>,
        draft: &mut CreateDraft<RejectingLifecycleUser>,
    ) -> Result<()> {
        if draft.pending_record().decode::<String>("email")? == "reject@example.com" {
            return Err(Error::message("creating hook rejected row"));
        }
        Ok(())
    }
}

struct PanickingLifecycleUserHooks;

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = USERS_TABLE, primary_key_strategy = "manual", lifecycle = PanickingLifecycleUserHooks)]
struct PanickingLifecycleUser {
    id: i64,
    email: String,
    active: bool,
    nickname: Option<String>,
}

#[async_trait]
impl ModelLifecycle<PanickingLifecycleUser> for PanickingLifecycleUserHooks {
    async fn creating(
        _context: &ModelHookContext<'_>,
        draft: &mut CreateDraft<PanickingLifecycleUser>,
    ) -> Result<()> {
        if draft.pending_record().text("email") == "panic-creating@example.com" {
            panic!("creating boom");
        }
        Ok(())
    }

    async fn created(
        _context: &ModelHookContext<'_>,
        _created: &PanickingLifecycleUser,
        record: &DbRecord,
    ) -> Result<()> {
        if record.text("email") == "panic-created@example.com" {
            panic!("created boom");
        }
        Ok(())
    }

    async fn updating(
        _context: &ModelHookContext<'_>,
        _current: &PanickingLifecycleUser,
        draft: &mut UpdateDraft<PanickingLifecycleUser>,
    ) -> Result<()> {
        if draft.pending_record().text("nickname") == "panic-updating" {
            panic!("updating boom");
        }
        Ok(())
    }

    async fn updated(
        _context: &ModelHookContext<'_>,
        _before: &PanickingLifecycleUser,
        _after: &PanickingLifecycleUser,
        _before_record: &DbRecord,
        after_record: &DbRecord,
    ) -> Result<()> {
        if after_record.text("nickname") == "panic-updated" {
            panic!("updated boom");
        }
        Ok(())
    }

    async fn deleting(
        _context: &ModelHookContext<'_>,
        _current: &PanickingLifecycleUser,
        record: &DbRecord,
    ) -> Result<()> {
        if record.text("email") == "panic-deleting@example.com" {
            panic!("deleting boom");
        }
        Ok(())
    }

    async fn deleted(
        _context: &ModelHookContext<'_>,
        _deleted: &PanickingLifecycleUser,
        record: &DbRecord,
    ) -> Result<()> {
        if record.text("email") == "panic-deleted@example.com" {
            panic!("deleted boom");
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, foundry::Model)]
#[foundry(table = SAFE_USERS_TABLE)]
struct SafeUuidUser {
    id: ModelId<SafeUuidUser>,
    email: String,
}

impl User {
    fn merchants() -> RelationDef<Self, Merchant> {
        has_many(
            Self::ID,
            Merchant::USER_ID,
            |user| user.id,
            |user, merchants| user.merchants = Loaded::new(merchants),
        )
    }

    fn active_merchant_count() -> RelationAggregateDef<Self, i64> {
        Self::merchants()
            .where_(Merchant::STATUS.eq(MerchantStatus::Active))
            .count(|user, count| user.merchant_count = Loaded::new(count))
    }
}

impl Merchant {
    fn orders() -> RelationDef<Self, Order> {
        has_many(
            Self::ID,
            Order::MERCHANT_ID,
            |merchant| merchant.id,
            |merchant, orders| merchant.orders = Loaded::new(orders),
        )
    }

    fn orders_total() -> RelationAggregateDef<Self, Option<i64>> {
        Self::orders().sum(Order::TOTAL, |merchant, total| {
            merchant.order_total = Loaded::new(total)
        })
    }

    fn tags() -> ManyToManyDef<Self, Tag, ()> {
        many_to_many(
            Self::ID,
            MERCHANT_TAGS_TABLE,
            "merchant_id",
            "tag_id",
            Tag::ID,
            |merchant| merchant.id,
            |merchant, tags| merchant.tags = Loaded::new(tags),
        )
    }

    fn tags_with_pivot() -> ManyToManyDef<Self, Tag, MerchantTagLink> {
        Self::tags().with_pivot(MerchantTagLink::projection_meta(), |tag, link| {
            tag.link = Loaded::new(link)
        })
    }

    fn tags_count() -> RelationAggregateDef<Self, i64> {
        Self::tags().count(|merchant, count| merchant.tag_count = Loaded::new(count))
    }
}

impl Order {
    fn items() -> RelationDef<Self, OrderItem> {
        has_many(
            Self::ID,
            OrderItem::ORDER_ID,
            |order| order.id,
            |order, items| order.items = Loaded::new(items),
        )
    }
}

impl OrderItem {
    fn product() -> RelationDef<Self, Product> {
        belongs_to(
            Self::PRODUCT_ID,
            Product::ID,
            |item| Some(item.product_id),
            |item, product| item.product = Loaded::new(product),
        )
    }
}

impl Tag {
    fn creator() -> RelationDef<Self, TagCreator> {
        belongs_to(
            Self::USER_ID,
            TagCreator::ID,
            |tag| Some(tag.user_id),
            |tag, creator| tag.creator = Loaded::new(creator),
        )
    }
}

#[tokio::test]
async fn raw_queries_and_transactions_work_against_postgres() {
    let Some(database) = test_database().await else {
        return;
    };
    let _guard = database_lock().lock().await;
    reset_schema(&database).await;

    database
        .raw_execute(
            &format!("INSERT INTO {USERS_TABLE} (id, email, active) VALUES ($1, $2, $3)"),
            &[1_i64.into(), "foundry@example.com".into(), true.into()],
        )
        .await
        .unwrap();

    let transaction = database.begin().await.unwrap();
    transaction
        .raw_execute(
            &format!("INSERT INTO {USERS_TABLE} (id, email, active) VALUES ($1, $2, $3)"),
            &[2_i64.into(), "rollback@example.com".into(), true.into()],
        )
        .await
        .unwrap();
    transaction.rollback().await.unwrap();

    let records = database
        .raw_query(&format!("SELECT COUNT(*) AS total FROM {USERS_TABLE}"), &[])
        .await
        .unwrap();

    assert_eq!(records[0].decode::<i64>("total").unwrap(), 1);
}

#[tokio::test]
async fn generic_builder_and_model_first_queries_are_typed_and_short() {
    let Some(runtime) = test_app_runtime().await else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    let generic_inserted = Query::insert_many_into(USERS_TABLE)
        .row([
            ("id", DbValue::from(1_i64)),
            ("email", DbValue::from("generic@example.com")),
            ("active", DbValue::from(true)),
            ("nickname", DbValue::Null(DbType::Text)),
        ])
        .row([
            ("email", DbValue::from("generic-two@example.com")),
            ("nickname", DbValue::from("ghost")),
            ("id", DbValue::from(4_i64)),
            ("active", DbValue::from(false)),
        ])
        .returning(["id", "email"])
        .get(database)
        .await
        .unwrap();
    assert_eq!(generic_inserted.len(), 2);

    let created = User::create()
        .set(User::ID, 2_i64)
        .set(User::EMAIL, "model@example.com")
        .set(User::ACTIVE, false)
        .set(User::NICKNAME, " Captain ")
        .save(app)
        .await
        .unwrap();

    let bulk_created = User::create_many()
        .row(|row| {
            row.set(User::ID, 3_i64)
                .set(User::EMAIL, "bulk-one@example.com")
                .set(User::ACTIVE, false)
                .set(User::NICKNAME, None::<String>)
        })
        .row(|row| {
            row.set(User::ID, 5_i64)
                .set(User::EMAIL, "bulk-two@example.com")
                .set(User::ACTIVE, true)
                .set(User::NICKNAME, " Ally ")
        })
        .get(app)
        .await
        .unwrap();

    assert_eq!(created.email, "model@example.com");
    assert!(!created.active);
    assert_eq!(created.nickname.as_deref(), Some("captain"));
    assert_eq!(bulk_created.len(), 2);
    assert_eq!(bulk_created[0].nickname, None);
    assert_eq!(bulk_created[1].nickname.as_deref(), Some("ally"));

    let ignored = Query::insert_into(USERS_TABLE)
        .value("id", 1_i64)
        .value("email", "ignored@example.com")
        .value("active", false)
        .on_conflict_columns(["id"])
        .do_nothing()
        .execute(database)
        .await
        .unwrap();
    assert_eq!(ignored, 0);

    let upserted = User::create()
        .set(User::ID, 3_i64)
        .set(User::EMAIL, "bulk-updated@example.com")
        .set(User::ACTIVE, true)
        .set(User::NICKNAME, "upserted")
        .on_conflict_columns([User::ID])
        .do_update()
        .set_excluded(User::EMAIL)
        .set_excluded(User::ACTIVE)
        .set_excluded(User::NICKNAME)
        .save(app)
        .await
        .unwrap();
    assert_eq!(upserted.email, "bulk-updated@example.com");
    assert!(upserted.active);
    assert_eq!(upserted.nickname.as_deref(), Some("upserted"));

    let generic = Query::table(USERS_TABLE)
        .select(["id", "email"])
        .where_eq("active", true)
        .order_by(OrderBy::asc("id"))
        .get(database)
        .await
        .unwrap();
    assert_eq!(generic.len(), 3);
    assert_eq!(
        generic[0].decode::<String>("email").unwrap(),
        "generic@example.com"
    );

    let paginated = User::query()
        .order_by(User::ID.asc())
        .paginate(database, Pagination::new(1, 10))
        .await
        .unwrap();
    assert_eq!(paginated.total, 5);
    assert_eq!(paginated.data.len(), 5);

    let total_users = User::query().count(database).await.unwrap();
    assert_eq!(total_users, 5);

    let all_users = User::query()
        .order_by(User::ID.asc())
        .all(database)
        .await
        .unwrap();
    assert_eq!(all_users.len(), 5);

    let found = User::query().find(database, 2_i64).await.unwrap().unwrap();
    assert_eq!(found.email, "model@example.com");

    let found_many = User::query()
        .order_by(User::ID.asc())
        .find_many(database, [2_i64, 5_i64])
        .await
        .unwrap();
    assert_eq!(
        found_many
            .iter()
            .map(|user| user.email.as_str())
            .collect::<Vec<_>>(),
        vec!["model@example.com", "bulk-two@example.com"]
    );

    let required = User::query()
        .where_(User::EMAIL.eq("bulk-one@example.com"))
        .first_or_fail(database)
        .await
        .unwrap();
    assert_eq!(required.id, 3);

    let required_by_key = User::query().find_or_fail(database, 4_i64).await.unwrap();
    assert_eq!(required_by_key.email, "generic-two@example.com");

    let missing_first = User::query()
        .where_(User::EMAIL.eq("missing@example.com"))
        .first_or_fail(database)
        .await
        .unwrap_err();
    assert!(format!("{missing_first:?}").contains("returned no records"));

    assert!(User::query()
        .where_(User::ACTIVE.eq(true))
        .exists(database)
        .await
        .unwrap());
    assert!(User::query()
        .where_(User::EMAIL.eq("missing@example.com"))
        .doesnt_exist(database)
        .await
        .unwrap());

    let model_email = User::query()
        .where_(User::ID.eq(2_i64))
        .value(database, User::EMAIL)
        .await
        .unwrap();
    assert_eq!(model_email.as_deref(), Some("model@example.com"));

    let chunked_ids = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Vec<i64>>::new()));
    User::query()
        .order_by(User::ID.asc())
        .chunk(database, 2, {
            let chunked_ids = chunked_ids.clone();
            move |users| {
                let chunked_ids = chunked_ids.clone();
                async move {
                    chunked_ids
                        .lock()
                        .unwrap()
                        .push(users.iter().map(|user| user.id).collect());
                    Ok::<(), Error>(())
                }
            }
        })
        .await
        .unwrap();
    assert_eq!(
        chunked_ids.lock().unwrap().clone(),
        vec![vec![1, 2], vec![3, 4], vec![5]]
    );

    let chunked_by_id = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Vec<i64>>::new()));
    User::query()
        .chunk_by_id(database, User::ID, 2, {
            let chunked_by_id = chunked_by_id.clone();
            move |users| {
                let chunked_by_id = chunked_by_id.clone();
                async move {
                    chunked_by_id
                        .lock()
                        .unwrap()
                        .push(users.iter().map(|user| user.id).collect());
                    Ok::<(), Error>(())
                }
            }
        })
        .await
        .unwrap();
    assert_eq!(
        chunked_by_id.lock().unwrap().clone(),
        vec![vec![1, 2], vec![3, 4], vec![5]]
    );

    let each_by_id = std::sync::Arc::new(std::sync::Mutex::new(Vec::<i64>::new()));
    User::query()
        .each_by_id(database, User::ID, 3, {
            let each_by_id = each_by_id.clone();
            move |user| {
                let each_by_id = each_by_id.clone();
                async move {
                    each_by_id.lock().unwrap().push(user.id);
                    Ok::<(), Error>(())
                }
            }
        })
        .await
        .unwrap();
    assert_eq!(each_by_id.lock().unwrap().clone(), vec![1, 2, 3, 4, 5]);

    let active_id_sum = User::query()
        .where_(User::ACTIVE.eq(true))
        .sum(database, User::ID)
        .await
        .unwrap();
    assert_eq!(active_id_sum, Some(9));

    let active_projection = AggregateProjection::<i64>::count_all("active_total");
    let aggregate_rows = Query::table(USERS_TABLE)
        .select(["active"])
        .select_aggregate(active_projection.clone())
        .group_by("active")
        .having(Condition::compare(
            Expr::Aggregate(AggregateExpr::count_all()),
            ComparisonOp::Gt,
            Expr::value(0_i64),
        ))
        .order_by(OrderBy::asc("active"))
        .get(database)
        .await
        .unwrap();
    assert_eq!(aggregate_rows.len(), 2);
    assert_eq!(active_projection.decode(&aggregate_rows[0]).unwrap(), 2);
    assert_eq!(active_projection.decode(&aggregate_rows[1]).unwrap(), 3);

    let updated = User::update()
        .set(User::ACTIVE, true)
        .set(User::NICKNAME, " VIP ")
        .where_(User::EMAIL.eq("bulk-two@example.com"))
        .save(app)
        .await
        .unwrap();
    assert!(updated.active);
    assert_eq!(updated.nickname.as_deref(), Some("vip"));

    let nulled = User::update()
        .set_null(User::NICKNAME)
        .where_(User::ID.eq(5_i64))
        .save(app)
        .await
        .unwrap();
    assert!(nulled.active);
    assert_eq!(nulled.nickname, None);

    let skipped = User::update()
        .set(User::ACTIVE, false)
        .where_(User::ID.eq(5_i64))
        .save(app)
        .await
        .unwrap();
    assert!(!skipped.active);
    assert_eq!(skipped.nickname, None);

    let fetched = User::query()
        .where_(User::ID.eq(5_i64))
        .first(database)
        .await
        .unwrap()
        .unwrap();
    let updated_from_instance = fetched
        .update()
        .set(User::ACTIVE, true)
        .set(User::NICKNAME, " Instance-Write ")
        .save(app)
        .await
        .unwrap();
    assert!(updated_from_instance.active);
    assert_eq!(
        updated_from_instance.nickname.as_deref(),
        Some("instance-write")
    );

    let deleted_rows = User::delete()
        .where_(User::EMAIL.eq("generic@example.com"))
        .execute(app)
        .await
        .unwrap();
    assert_eq!(deleted_rows, 1);
}

#[tokio::test]
async fn manual_primary_keys_require_explicit_values_on_create() {
    let Some(runtime) = test_app_runtime().await else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    let error = User::create()
        .set(User::EMAIL, "missing-id@example.com")
        .set(User::ACTIVE, true)
        .save(app)
        .await
        .unwrap_err();

    assert!(
        error.to_string().contains("primary_key_strategy is manual"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn text_columns_support_case_insensitive_exact_match_without_pattern_wildcards() {
    let Some(database) = test_database().await else {
        return;
    };
    let _guard = database_lock().lock().await;
    reset_schema(&database).await;

    execute_batch(
        &database,
        &[
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active, nickname) VALUES (1, 'Exact%Match@Example.com', true, 'VIP')"
            ),
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active, nickname) VALUES (2, 'ExactXMatch@Example.com', true, 'Regular')"
            ),
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active, nickname) VALUES (3, 'someone@example.com', true, NULL)"
            ),
        ],
    )
    .await;

    let typed = User::query()
        .where_(User::EMAIL.ieq("exact%match@example.com"))
        .get(&database)
        .await
        .unwrap();
    assert_eq!(typed.len(), 1);
    assert_eq!(typed[0].id, 1);
    assert_eq!(typed[0].email, "Exact%Match@Example.com");

    let generic = Query::table(USERS_TABLE)
        .select(["id", "email"])
        .where_ieq("email", "EXACT%MATCH@EXAMPLE.COM")
        .get(&database)
        .await
        .unwrap();
    assert_eq!(generic.len(), 1);
    assert_eq!(generic[0].decode::<i64>("id").unwrap(), 1);
    assert_eq!(
        generic[0].decode::<String>("email").unwrap(),
        "Exact%Match@Example.com"
    );

    let nullable = User::query()
        .where_(User::NICKNAME.ieq("vip"))
        .get(&database)
        .await
        .unwrap();
    assert_eq!(nullable.len(), 1);
    assert_eq!(nullable[0].id, 1);
    assert_eq!(nullable[0].nickname.as_deref(), Some("VIP"));
}

#[tokio::test]
async fn safe_model_ids_auto_generate_and_sort_newest_first() {
    let Some(runtime) = test_app_runtime().await else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;

    database
        .raw_execute(&format!("DROP TABLE IF EXISTS {SAFE_USERS_TABLE}"), &[])
        .await
        .unwrap();
    database
        .raw_execute(
            &format!(
                "CREATE TABLE {SAFE_USERS_TABLE} (id UUID PRIMARY KEY DEFAULT uuidv7(), email TEXT NOT NULL)"
            ),
            &[],
        )
        .await
        .unwrap();

    let first = SafeUuidUser::create()
        .set(SafeUuidUser::EMAIL, "first@example.com")
        .save(app)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    let second = SafeUuidUser::create()
        .set(SafeUuidUser::EMAIL, "second@example.com")
        .save(app)
        .await
        .unwrap();

    let ordered = SafeUuidUser::query()
        .order_by(SafeUuidUser::ID.desc())
        .get(database)
        .await
        .unwrap();

    assert_eq!(ordered.len(), 2);
    assert_eq!(ordered[0].id, second.id);
    assert_eq!(ordered[1].id, first.id);
    assert_ne!(first.id, second.id);
}

#[tokio::test]
async fn model_write_mutators_hash_passwords_and_generated_accessors_return_transformed_values() {
    let Some(runtime) = test_app_runtime().await else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    let created = PasswordUser::create()
        .set(PasswordUser::ID, 1_i64)
        .set(PasswordUser::EMAIL, "foundry@example.com")
        .set(PasswordUser::PASSWORD, "secret-password")
        .save(app)
        .await
        .unwrap();

    assert_ne!(created.password, "secret-password");
    assert!(app
        .hash()
        .unwrap()
        .check("secret-password", &created.password)
        .unwrap());
    assert_eq!(created.password_accessed(), "********");
    assert_eq!(created.password, created.clone().password);

    let bulk_created = PasswordUser::create_many()
        .row(|row| {
            row.set(PasswordUser::ID, 2_i64)
                .set(PasswordUser::EMAIL, "ops@example.com")
                .set(PasswordUser::PASSWORD, "ops-secret")
        })
        .row(|row| {
            row.set(PasswordUser::ID, 3_i64)
                .set(PasswordUser::EMAIL, "dev@example.com")
                .set(PasswordUser::PASSWORD, "dev-secret")
        })
        .get(app)
        .await
        .unwrap();

    assert_eq!(bulk_created.len(), 2);
    assert!(app
        .hash()
        .unwrap()
        .check("ops-secret", &bulk_created[0].password)
        .unwrap());
    assert!(app
        .hash()
        .unwrap()
        .check("dev-secret", &bulk_created[1].password)
        .unwrap());

    let updated = created
        .update()
        .set(PasswordUser::PASSWORD, "new-secret")
        .save(app)
        .await
        .unwrap();
    assert!(app
        .hash()
        .unwrap()
        .check("new-secret", &updated.password)
        .unwrap());

    PasswordUser::update()
        .set(PasswordUser::PASSWORD, "bulk-secret")
        .where_(PasswordUser::ID.eq(2_i64))
        .without_lifecycle()
        .execute(app)
        .await
        .unwrap();

    let fast_updated = PasswordUser::query()
        .where_(PasswordUser::ID.eq(2_i64))
        .first(database)
        .await
        .unwrap()
        .unwrap();
    assert!(app
        .hash()
        .unwrap()
        .check("bulk-secret", &fast_updated.password)
        .unwrap());

    database
        .raw_execute(
            &format!(
                "INSERT INTO {PASSWORD_USERS_TABLE} (id, email, password) VALUES ($1, $2, $3)"
            ),
            &[
                DbValue::from(4_i64),
                DbValue::from("raw@example.com"),
                DbValue::from("raw-password"),
            ],
        )
        .await
        .unwrap();

    let raw_inserted = PasswordUser::query()
        .where_(PasswordUser::ID.eq(4_i64))
        .first(database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(raw_inserted.password, "raw-password");
}

#[tokio::test]
async fn write_mutator_panics_roll_back_model_writes() {
    let Some(runtime) = test_app_runtime().await else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    let create_error = PanickingPasswordUser::create()
        .set(PanickingPasswordUser::ID, 10_i64)
        .set(PanickingPasswordUser::EMAIL, "panic-create@example.com")
        .set(PanickingPasswordUser::PASSWORD, "panic-create")
        .save(app)
        .await
        .unwrap_err();
    assert!(create_error
        .to_string()
        .contains("write mutator `password` panicked: password mutator boom"));
    assert!(PanickingPasswordUser::query()
        .where_(PanickingPasswordUser::ID.eq(10_i64))
        .first(database)
        .await
        .unwrap()
        .is_none());

    let bulk_error = PanickingPasswordUser::create_many()
        .row(|row| {
            row.set(PanickingPasswordUser::ID, 11_i64)
                .set(PanickingPasswordUser::EMAIL, "bulk-ok@example.com")
                .set(PanickingPasswordUser::PASSWORD, "bulk-ok")
        })
        .row(|row| {
            row.set(PanickingPasswordUser::ID, 12_i64)
                .set(PanickingPasswordUser::EMAIL, "bulk-panic@example.com")
                .set(PanickingPasswordUser::PASSWORD, "panic-create")
        })
        .execute(app)
        .await
        .unwrap_err();
    assert!(bulk_error
        .to_string()
        .contains("write mutator `password` panicked: password mutator boom"));
    assert!(PanickingPasswordUser::query()
        .where_(Condition::or([
            PanickingPasswordUser::ID.eq(11_i64),
            PanickingPasswordUser::ID.eq(12_i64),
        ]))
        .get(database)
        .await
        .unwrap()
        .is_empty());

    let created = PanickingPasswordUser::create()
        .set(PanickingPasswordUser::ID, 13_i64)
        .set(PanickingPasswordUser::EMAIL, "ok@example.com")
        .set(PanickingPasswordUser::PASSWORD, "ok")
        .save(app)
        .await
        .unwrap();
    assert_eq!(created.password, "mutated:ok");

    let update_error = PanickingPasswordUser::update()
        .set(PanickingPasswordUser::PASSWORD, "panic-update")
        .where_(PanickingPasswordUser::ID.eq(13_i64))
        .execute(app)
        .await
        .unwrap_err();
    assert!(update_error
        .to_string()
        .contains("write mutator `password` panicked: password mutator boom"));
    let unchanged = PanickingPasswordUser::query()
        .where_(PanickingPasswordUser::ID.eq(13_i64))
        .first(database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.password, "mutated:ok");

    let conflict_error = PanickingPasswordUser::create()
        .set(PanickingPasswordUser::ID, 13_i64)
        .set(PanickingPasswordUser::EMAIL, "conflict@example.com")
        .set(PanickingPasswordUser::PASSWORD, "incoming")
        .on_conflict_columns([PanickingPasswordUser::ID])
        .do_update()
        .set_conflict(PanickingPasswordUser::PASSWORD, "panic-update")
        .save(app)
        .await
        .unwrap_err();
    assert!(conflict_error
        .to_string()
        .contains("write mutator `password` panicked: password mutator boom"));
    let unchanged = PanickingPasswordUser::query()
        .where_(PanickingPasswordUser::ID.eq(13_i64))
        .first(database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.email, "ok@example.com");
    assert_eq!(unchanged.password, "mutated:ok");
}

#[tokio::test]
async fn timestamps_and_soft_deletes_work_on_model_first_writes() {
    let Some(runtime) = test_app_runtime().await else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    let created = PostRecord::create()
        .set(PostRecord::ID, 1_i64)
        .set(PostRecord::TITLE, "Hello")
        .set(PostRecord::BODY, "World")
        .save(app)
        .await
        .unwrap();
    assert!(created.deleted_at.is_none());
    assert!(created.updated_at >= created.created_at);

    tokio::time::sleep(Duration::from_millis(5)).await;
    let updated = created
        .update()
        .set(PostRecord::TITLE, "Updated")
        .save(app)
        .await
        .unwrap();
    assert!(updated.updated_at > created.updated_at);

    tokio::time::sleep(Duration::from_millis(5)).await;
    let fast_updated = PostRecord::update()
        .set(PostRecord::BODY, "Fast path")
        .where_(PostRecord::ID.eq(updated.id))
        .without_lifecycle()
        .save(app)
        .await
        .unwrap();
    assert!(fast_updated.updated_at > updated.updated_at);

    let deleted = fast_updated.delete().execute(app).await.unwrap();
    assert_eq!(deleted, 1);

    let visible = PostRecord::query().get(database).await.unwrap();
    assert!(visible.is_empty());

    let all = PostRecord::query()
        .with_trashed()
        .get(database)
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].deleted_at.is_some());

    let trashed = PostRecord::query()
        .only_trashed()
        .get(database)
        .await
        .unwrap();
    assert_eq!(trashed.len(), 1);

    let restored = PostRecord::restore()
        .where_(PostRecord::ID.eq(1_i64))
        .save(app)
        .await
        .unwrap();
    assert!(restored.deleted_at.is_none());

    let forced = PostRecord::force_delete()
        .where_(PostRecord::ID.eq(1_i64))
        .execute(app)
        .await
        .unwrap();
    assert_eq!(forced, 1);
    assert!(PostRecord::query()
        .with_trashed()
        .get(database)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn nullable_timestamp_comparison_excludes_nulls_and_future_values() {
    let Some(runtime) = test_app_runtime().await else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    let now = fixed_time(15);
    let past = fixed_time(14);
    let future = fixed_time(16);

    let compiled = OptOutSoftDeletePost::query()
        .where_(Condition::or([
            OptOutSoftDeletePost::DELETED_AT.is_null(),
            OptOutSoftDeletePost::DELETED_AT.lte(now),
        ]))
        .to_compiled_sql()
        .unwrap();
    assert!(compiled.sql.contains(
        "(\"foundry_test_posts\".\"deleted_at\" IS NULL OR \"foundry_test_posts\".\"deleted_at\" <= $1::timestamptz)"
    ));
    assert_eq!(compiled.bindings, vec![DbValue::TimestampTz(now)]);

    OptOutSoftDeletePost::create()
        .set(OptOutSoftDeletePost::ID, 30_i64)
        .set(OptOutSoftDeletePost::TITLE, "No window")
        .set(OptOutSoftDeletePost::BODY, "Null deleted_at")
        .set(OptOutSoftDeletePost::DELETED_AT, None::<DateTime>)
        .save(app)
        .await
        .unwrap();
    OptOutSoftDeletePost::create()
        .set(OptOutSoftDeletePost::ID, 31_i64)
        .set(OptOutSoftDeletePost::TITLE, "Past")
        .set(OptOutSoftDeletePost::BODY, "Past deleted_at")
        .set(OptOutSoftDeletePost::DELETED_AT, past)
        .save(app)
        .await
        .unwrap();
    OptOutSoftDeletePost::create()
        .set(OptOutSoftDeletePost::ID, 32_i64)
        .set(OptOutSoftDeletePost::TITLE, "Future")
        .set(OptOutSoftDeletePost::BODY, "Future deleted_at")
        .set(OptOutSoftDeletePost::DELETED_AT, future)
        .save(app)
        .await
        .unwrap();

    let expired = OptOutSoftDeletePost::query()
        .where_(OptOutSoftDeletePost::DELETED_AT.lte(now))
        .order_by(OptOutSoftDeletePost::ID.asc())
        .get(database)
        .await
        .unwrap();
    assert_eq!(
        expired.iter().map(|post| post.id).collect::<Vec<_>>(),
        vec![31_i64]
    );

    let nullable_or_expired = OptOutSoftDeletePost::query()
        .where_(Condition::or([
            OptOutSoftDeletePost::DELETED_AT.is_null(),
            OptOutSoftDeletePost::DELETED_AT.lte(now),
        ]))
        .order_by(OptOutSoftDeletePost::ID.asc())
        .get(database)
        .await
        .unwrap();
    assert_eq!(
        nullable_or_expired
            .iter()
            .map(|post| post.id)
            .collect::<Vec<_>>(),
        vec![30_i64, 31_i64]
    );
}

#[tokio::test]
async fn numeric_models_and_raw_queries_support_postgres_numeric_columns() {
    let Some(runtime) = test_app_runtime().await else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    let created = NumericPostRecord::create()
        .set(NumericPostRecord::ID, 1_i64)
        .set(
            NumericPostRecord::AMOUNT,
            Numeric::new("10.50000000").unwrap(),
        )
        .set(NumericPostRecord::NOTE, "starter balance")
        .save(app)
        .await
        .unwrap();
    assert_eq!(created.amount.as_str(), "10.50000000");
    assert!(created.deleted_at.is_none());

    let fetched = NumericPostRecord::query()
        .where_(NumericPostRecord::ID.eq(1_i64))
        .first(database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.amount.as_str(), "10.50000000");

    let amount_only = database
        .raw_query(
            &format!("SELECT amount FROM {NUMERIC_POSTS_TABLE} WHERE id = $1 ORDER BY id LIMIT 1"),
            &[1_i64.into()],
        )
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        amount_only.decode::<Numeric>("amount").unwrap().as_str(),
        "10.50000000"
    );

    let full_row = database
        .raw_query(
            &format!("SELECT * FROM {NUMERIC_POSTS_TABLE} WHERE id = $1"),
            &[1_i64.into()],
        )
        .await
        .unwrap()
        .remove(0);
    assert_eq!(full_row.decode::<i64>("id").unwrap(), 1);
    assert_eq!(
        full_row.decode::<Numeric>("amount").unwrap().as_str(),
        "10.50000000"
    );
    assert_eq!(
        full_row.decode::<String>("note").unwrap(),
        "starter balance".to_string()
    );

    let numeric_array = database
        .raw_query(
            &format!(
                "SELECT ARRAY[amount, amount + 1]::numeric[] AS amounts FROM {NUMERIC_POSTS_TABLE} WHERE id = $1"
            ),
            &[1_i64.into()],
        )
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        numeric_array
            .decode::<Vec<Numeric>>("amounts")
            .unwrap()
            .iter()
            .map(Numeric::as_str)
            .collect::<Vec<_>>(),
        vec!["10.50000000", "11.50000000"]
    );

    let updated = fetched
        .update()
        .set(
            NumericPostRecord::AMOUNT,
            Numeric::new("12.75000000").unwrap(),
        )
        .set(NumericPostRecord::NOTE, "updated balance")
        .save(app)
        .await
        .unwrap();
    assert_eq!(updated.amount.as_str(), "12.75000000");
    assert_eq!(updated.note, "updated balance");

    let deleted = updated.delete().execute(app).await.unwrap();
    assert_eq!(deleted, 1);

    let trashed = NumericPostRecord::query()
        .with_trashed()
        .where_(NumericPostRecord::ID.eq(1_i64))
        .first(database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(trashed.amount.as_str(), "12.75000000");
    assert!(trashed.deleted_at.is_some());

    let forced = NumericPostRecord::force_delete()
        .where_(NumericPostRecord::ID.eq(1_i64))
        .execute(app)
        .await
        .unwrap();
    assert_eq!(forced, 1);
    assert!(NumericPostRecord::query()
        .with_trashed()
        .get(database)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn app_config_can_disable_default_timestamp_management() {
    let Some(runtime) = test_app_runtime_with_provider_and_config(
        NoopProvider,
        r#"
            [database.models]
            timestamps_default = false
        "#,
    )
    .await
    else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    let manual_created_at = fixed_time(10);
    let manual_updated_at = fixed_time(11);

    let created = PostRecord::create()
        .set(PostRecord::ID, 2_i64)
        .set(PostRecord::TITLE, "Manual timestamps")
        .set(PostRecord::BODY, "Still works")
        .set(PostRecord::CREATED_AT, manual_created_at)
        .set(PostRecord::UPDATED_AT, manual_updated_at)
        .save(app)
        .await
        .unwrap();
    assert_eq!(created.created_at, manual_created_at);
    assert_eq!(created.updated_at, manual_updated_at);

    let updated = created
        .update()
        .set(PostRecord::TITLE, "No auto bump")
        .save(app)
        .await
        .unwrap();
    assert_eq!(updated.updated_at, manual_updated_at);
}

#[tokio::test]
async fn app_config_can_enable_default_soft_delete_behavior_and_model_can_opt_out() {
    let _guard = database_lock().lock().await;
    let Some(runtime) = test_app_runtime_with_provider_and_config(
        NoopProvider,
        r#"
            [database.models]
            soft_deletes_default = true
        "#,
    )
    .await
    else {
        return;
    };
    let app = &runtime.app;
    let database = &runtime.database;

    reset_schema(database).await;

    let created = DefaultSoftDeletePost::create()
        .set(DefaultSoftDeletePost::ID, 20_i64)
        .set(DefaultSoftDeletePost::TITLE, "config soft delete")
        .set(DefaultSoftDeletePost::BODY, "enabled")
        .save(app)
        .await
        .unwrap();

    created.delete().execute(app).await.unwrap();

    assert!(DefaultSoftDeletePost::query()
        .where_(DefaultSoftDeletePost::ID.eq(20_i64))
        .first(database.as_ref())
        .await
        .unwrap()
        .is_none());

    assert!(DefaultSoftDeletePost::query()
        .with_trashed()
        .where_(DefaultSoftDeletePost::ID.eq(20_i64))
        .first(database.as_ref())
        .await
        .unwrap()
        .is_some());

    let created = OptOutSoftDeletePost::create()
        .set(OptOutSoftDeletePost::ID, 21_i64)
        .set(OptOutSoftDeletePost::TITLE, "opt out")
        .set(OptOutSoftDeletePost::BODY, "hard delete")
        .save(app)
        .await
        .unwrap();

    created.delete().execute(app).await.unwrap();

    assert!(OptOutSoftDeletePost::query()
        .with_trashed()
        .where_(OptOutSoftDeletePost::ID.eq(21_i64))
        .first(database.as_ref())
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn built_in_timestamp_values_are_visible_to_hooks_and_soft_delete_stays_on_delete_hooks() {
    let log = std::sync::Arc::new(TimestampHookLog::default());
    let Some(runtime) =
        test_app_runtime_with_provider(TimestampHookProvider { log: log.clone() }).await
    else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    let created = HookedPost::create()
        .set(HookedPost::ID, 3_i64)
        .set(HookedPost::TITLE, "Hooked")
        .set(HookedPost::BODY, "Create")
        .save(app)
        .await
        .unwrap();
    assert_eq!(created.updated_at, fixed_time(2));
    assert_eq!(log.snapshot().await, vec!["creating:true:true"]);

    log.clear().await;
    let updated = created
        .update()
        .set(HookedPost::BODY, "Update")
        .save(app)
        .await
        .unwrap();
    assert_eq!(updated.updated_at, fixed_time(3));
    assert_eq!(log.snapshot().await, vec!["updating:true"]);

    log.clear().await;
    let deleted = HookedPost::delete()
        .where_(HookedPost::ID.eq(3_i64))
        .execute(app)
        .await
        .unwrap();
    assert_eq!(deleted, 1);
    assert_eq!(log.snapshot().await, vec!["deleting", "deleted"]);

    let trashed = HookedPost::query()
        .with_trashed()
        .first(database)
        .await
        .unwrap()
        .unwrap();
    assert!(trashed.deleted_at.is_some());
}

#[tokio::test]
async fn model_lifecycle_hooks_and_framework_events_run_automatically() {
    let log = std::sync::Arc::new(LifecycleLog::default());
    let Some(runtime) = test_app_runtime_with_provider(LifecycleTestProvider {
        log: log.clone(),
        fail_created: false,
    })
    .await
    else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    let created = LifecycleUser::create()
        .set(LifecycleUser::ID, 90_i64)
        .set(LifecycleUser::EMAIL, "lifecycle@example.com")
        .set(LifecycleUser::ACTIVE, true)
        .save(app)
        .await
        .unwrap();
    assert_eq!(created.nickname.as_deref(), Some("hook-created"));

    let fetched = LifecycleUser::query()
        .where_(LifecycleUser::ID.eq(90_i64))
        .first(database)
        .await
        .unwrap()
        .unwrap();
    let updated = fetched
        .update()
        .set(LifecycleUser::ACTIVE, false)
        .save(app)
        .await
        .unwrap();
    assert_eq!(updated.nickname.as_deref(), Some("hook-updated"));

    let deleted = LifecycleUser::delete()
        .where_(LifecycleUser::ID.eq(90_i64))
        .execute(app)
        .await
        .unwrap();
    assert_eq!(deleted, 1);

    assert_eq!(
        log.snapshot().await,
        vec![
            "hook:creating",
            "custom:creating",
            "event:creating",
            "hook:created",
            "event:created",
            "hook:updating",
            "custom:updating",
            "event:updating",
            "hook:updated",
            "event:updated",
            "hook:deleting",
            "event:deleting",
            "hook:deleted",
            "event:deleted",
        ]
    );
}

#[tokio::test]
async fn model_created_events_run_after_commit_for_fk_safe_listeners() {
    let Some(runtime) = test_app_runtime_with_provider(ProfileOnUserCreatedProvider).await else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    LifecycleUser::create()
        .set(LifecycleUser::ID, 95_i64)
        .set(LifecycleUser::EMAIL, "post-commit@example.com")
        .set(LifecycleUser::ACTIVE, true)
        .save(app)
        .await
        .unwrap();

    let profile_count = Query::table(USER_PROFILES_TABLE)
        .select_expr(Expr::Aggregate(AggregateExpr::count_all()), "count")
        .where_(Condition::raw("user_id = ?", vec![DbValue::from(95_i64)]))
        .first(database)
        .await
        .unwrap()
        .unwrap()
        .decode::<i64>("count")
        .unwrap();
    assert_eq!(profile_count, 1);

    let tx = app.begin_transaction().await.unwrap();
    LifecycleUser::create()
        .set(LifecycleUser::ID, 96_i64)
        .set(LifecycleUser::EMAIL, "explicit-transaction@example.com")
        .set(LifecycleUser::ACTIVE, true)
        .save(&tx)
        .await
        .unwrap();

    let profile_count_before_commit = Query::table(USER_PROFILES_TABLE)
        .select_expr(Expr::Aggregate(AggregateExpr::count_all()), "count")
        .where_(Condition::raw("user_id = ?", vec![DbValue::from(96_i64)]))
        .first(database)
        .await
        .unwrap()
        .unwrap()
        .decode::<i64>("count")
        .unwrap();
    assert_eq!(profile_count_before_commit, 0);

    tx.commit().await.unwrap();

    let profile_count_after_commit = Query::table(USER_PROFILES_TABLE)
        .select_expr(Expr::Aggregate(AggregateExpr::count_all()), "count")
        .where_(Condition::raw("user_id = ?", vec![DbValue::from(96_i64)]))
        .first(database)
        .await
        .unwrap()
        .unwrap()
        .decode::<i64>("count")
        .unwrap();
    assert_eq!(profile_count_after_commit, 1);
}

#[tokio::test]
async fn bulk_model_writes_default_to_lifecycle_and_without_lifecycle_skips_it() {
    let log = std::sync::Arc::new(LifecycleLog::default());
    let Some(runtime) = test_app_runtime_with_provider(LifecycleTestProvider {
        log: log.clone(),
        fail_created: false,
    })
    .await
    else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    let created = LifecycleUser::create_many()
        .row(|row| {
            row.set(LifecycleUser::ID, 100_i64)
                .set(LifecycleUser::EMAIL, "bulk-one@example.com")
                .set(LifecycleUser::ACTIVE, true)
        })
        .row(|row| {
            row.set(LifecycleUser::ID, 101_i64)
                .set(LifecycleUser::EMAIL, "bulk-two@example.com")
                .set(LifecycleUser::ACTIVE, true)
        })
        .get(app)
        .await
        .unwrap();
    assert_eq!(created.len(), 2);
    assert!(created
        .iter()
        .all(|user| user.nickname.as_deref() == Some("hook-created")));
    assert_eq!(
        log.snapshot().await,
        vec![
            "hook:creating",
            "custom:creating",
            "event:creating",
            "hook:created",
            "hook:creating",
            "custom:creating",
            "event:creating",
            "hook:created",
            "event:created",
            "event:created",
        ]
    );

    log.clear().await;
    let updated = LifecycleUser::update()
        .set(LifecycleUser::ACTIVE, false)
        .allow_all()
        .execute(app)
        .await
        .unwrap();
    assert_eq!(updated, 2);
    assert_eq!(
        log.snapshot().await,
        vec![
            "hook:updating",
            "custom:updating",
            "event:updating",
            "hook:updated",
            "hook:updating",
            "custom:updating",
            "event:updating",
            "hook:updated",
            "event:updated",
            "event:updated",
        ]
    );

    log.clear().await;
    let skipped = LifecycleUser::update()
        .set(LifecycleUser::NICKNAME, "fast-path")
        .allow_all()
        .without_lifecycle()
        .execute(app)
        .await
        .unwrap();
    assert_eq!(skipped, 2);
    assert!(log.snapshot().await.is_empty());

    let rows = LifecycleUser::query()
        .order_by(LifecycleUser::ID.asc())
        .get(database)
        .await
        .unwrap();
    assert!(rows
        .iter()
        .all(|row| row.nickname.as_deref() == Some("fast-path")));

    log.clear().await;
    let fast_created = LifecycleUser::create_many()
        .row(|row| {
            row.set(LifecycleUser::ID, 102_i64)
                .set(LifecycleUser::EMAIL, "fast-bulk-one@example.com")
                .set(LifecycleUser::ACTIVE, true)
        })
        .row(|row| {
            row.set(LifecycleUser::ID, 103_i64)
                .set(LifecycleUser::EMAIL, "fast-bulk-two@example.com")
                .set(LifecycleUser::ACTIVE, true)
        })
        .without_lifecycle()
        .get(app)
        .await
        .unwrap();
    assert_eq!(fast_created.len(), 2);
    assert!(fast_created.iter().all(|user| user.nickname.is_none()));
    assert!(log.snapshot().await.is_empty());
}

#[tokio::test]
async fn lifecycle_pre_commit_failures_roll_back_and_post_commit_failures_do_not() {
    let Some(runtime) = test_app_runtime().await else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    let hook_error = RejectingLifecycleUser::create()
        .set(RejectingLifecycleUser::ID, 120_i64)
        .set(RejectingLifecycleUser::EMAIL, "reject@example.com")
        .set(RejectingLifecycleUser::ACTIVE, true)
        .save(app)
        .await
        .unwrap_err();
    assert!(hook_error
        .to_string()
        .contains("creating hook rejected row"));

    let count_after_hook_failure = Query::table(USERS_TABLE)
        .select_expr(Expr::Aggregate(AggregateExpr::count_all()), "count")
        .first(database)
        .await
        .unwrap()
        .unwrap()
        .decode::<i64>("count")
        .unwrap();
    assert_eq!(count_after_hook_failure, 0);

    let log = std::sync::Arc::new(LifecycleLog::default());
    let Some(failing_runtime) = test_app_runtime_with_provider(LifecycleTestProvider {
        log,
        fail_created: true,
    })
    .await
    else {
        return;
    };
    let database = failing_runtime.database.as_ref();
    let app = &failing_runtime.app;
    reset_schema(database).await;

    let created = LifecycleUser::create()
        .set(LifecycleUser::ID, 121_i64)
        .set(LifecycleUser::EMAIL, "event-fail@example.com")
        .set(LifecycleUser::ACTIVE, true)
        .save(app)
        .await
        .unwrap();
    assert_eq!(created.id, 121_i64);

    let count_after_event_failure = Query::table(USERS_TABLE)
        .select_expr(Expr::Aggregate(AggregateExpr::count_all()), "count")
        .first(database)
        .await
        .unwrap()
        .unwrap()
        .decode::<i64>("count")
        .unwrap();
    assert_eq!(count_after_event_failure, 1);
}

#[tokio::test]
async fn model_lifecycle_panics_roll_back_like_hook_errors() {
    let Some(runtime) = test_app_runtime().await else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    let creating_error = PanickingLifecycleUser::create()
        .set(PanickingLifecycleUser::ID, 130_i64)
        .set(PanickingLifecycleUser::EMAIL, "panic-creating@example.com")
        .set(PanickingLifecycleUser::ACTIVE, true)
        .save(app)
        .await
        .unwrap_err();
    assert!(creating_error
        .to_string()
        .contains("creating hook panicked: creating boom"));
    assert!(PanickingLifecycleUser::query()
        .where_(PanickingLifecycleUser::ID.eq(130_i64))
        .first(database)
        .await
        .unwrap()
        .is_none());

    let created_error = PanickingLifecycleUser::create()
        .set(PanickingLifecycleUser::ID, 131_i64)
        .set(PanickingLifecycleUser::EMAIL, "panic-created@example.com")
        .set(PanickingLifecycleUser::ACTIVE, true)
        .save(app)
        .await
        .unwrap_err();
    assert!(created_error
        .to_string()
        .contains("created hook panicked: created boom"));
    assert!(PanickingLifecycleUser::query()
        .where_(PanickingLifecycleUser::ID.eq(131_i64))
        .first(database)
        .await
        .unwrap()
        .is_none());

    PanickingLifecycleUser::create()
        .set(PanickingLifecycleUser::ID, 132_i64)
        .set(
            PanickingLifecycleUser::EMAIL,
            "panic-update-target@example.com",
        )
        .set(PanickingLifecycleUser::ACTIVE, true)
        .save(app)
        .await
        .unwrap();

    let updating_error = PanickingLifecycleUser::update()
        .set(PanickingLifecycleUser::NICKNAME, "panic-updating")
        .where_(PanickingLifecycleUser::ID.eq(132_i64))
        .execute(app)
        .await
        .unwrap_err();
    assert!(updating_error
        .to_string()
        .contains("updating hook panicked: updating boom"));
    let update_target = PanickingLifecycleUser::query()
        .where_(PanickingLifecycleUser::ID.eq(132_i64))
        .first(database)
        .await
        .unwrap()
        .unwrap();
    assert!(update_target.nickname.is_none());

    let updated_error = PanickingLifecycleUser::update()
        .set(PanickingLifecycleUser::NICKNAME, "panic-updated")
        .where_(PanickingLifecycleUser::ID.eq(132_i64))
        .execute(app)
        .await
        .unwrap_err();
    assert!(updated_error
        .to_string()
        .contains("updated hook panicked: updated boom"));
    let update_target = PanickingLifecycleUser::query()
        .where_(PanickingLifecycleUser::ID.eq(132_i64))
        .first(database)
        .await
        .unwrap()
        .unwrap();
    assert!(update_target.nickname.is_none());

    PanickingLifecycleUser::create()
        .set(PanickingLifecycleUser::ID, 133_i64)
        .set(PanickingLifecycleUser::EMAIL, "panic-deleting@example.com")
        .set(PanickingLifecycleUser::ACTIVE, true)
        .save(app)
        .await
        .unwrap();
    let deleting_error = PanickingLifecycleUser::delete()
        .where_(PanickingLifecycleUser::ID.eq(133_i64))
        .execute(app)
        .await
        .unwrap_err();
    assert!(deleting_error
        .to_string()
        .contains("deleting hook panicked: deleting boom"));
    assert!(PanickingLifecycleUser::query()
        .where_(PanickingLifecycleUser::ID.eq(133_i64))
        .first(database)
        .await
        .unwrap()
        .is_some());

    PanickingLifecycleUser::create()
        .set(PanickingLifecycleUser::ID, 134_i64)
        .set(PanickingLifecycleUser::EMAIL, "panic-deleted@example.com")
        .set(PanickingLifecycleUser::ACTIVE, true)
        .save(app)
        .await
        .unwrap();
    let deleted_error = PanickingLifecycleUser::delete()
        .where_(PanickingLifecycleUser::ID.eq(134_i64))
        .execute(app)
        .await
        .unwrap_err();
    assert!(deleted_error
        .to_string()
        .contains("deleted hook panicked: deleted boom"));
    assert!(PanickingLifecycleUser::query()
        .where_(PanickingLifecycleUser::ID.eq(134_i64))
        .first(database)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn relation_tree_eager_loads_without_hardcoded_depth() {
    let Some(runtime) = test_app_runtime().await else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    execute_batch(
        database,
        &[
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active) VALUES (1, 'owner@example.com', true)"
            ),
            &format!(
                "INSERT INTO {MERCHANTS_TABLE} (id, user_id, name, status) VALUES (10, 1, 'Foundry Store', 'active')"
            ),
            &format!(
                "INSERT INTO {MERCHANTS_TABLE} (id, user_id, name, status) VALUES (11, 1, 'Archived Store', 'suspended')"
            ),
            &format!("INSERT INTO {ORDERS_TABLE} (id, merchant_id, total) VALUES (20, 10, 2500)"),
            &format!("INSERT INTO {PRODUCTS_TABLE} (id, name) VALUES (30, 'Foundry Mug')"),
            &format!(
                "INSERT INTO {ORDER_ITEMS_TABLE} (id, order_id, product_id, quantity) VALUES (40, 20, 30, 2)"
            ),
        ],
    )
    .await;

    let users = User::query()
        .with_aggregate(User::active_merchant_count())
        .with(
            User::merchants()
                .with_aggregate(Merchant::orders_total())
                .with(Merchant::orders().with(Order::items().with(OrderItem::product()))),
        )
        .where_has(User::merchants(), |query| {
            query.where_(Merchant::STATUS.eq(MerchantStatus::Active))
        })
        .get(database)
        .await
        .unwrap();

    assert_eq!(users.len(), 1);
    let user = &users[0];
    assert_eq!(user.merchant_count.as_ref(), Some(&1));
    let merchants = user.merchants.as_ref().unwrap();
    assert_eq!(merchants.len(), 1);
    assert_eq!(merchants[0].name, "Foundry Store");
    assert_eq!(merchants[0].status, MerchantStatus::Active);
    assert_eq!(merchants[0].order_total.as_ref(), Some(&Some(2500)));

    let orders = merchants[0].orders.as_ref().unwrap();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].total, 2500);

    let items = orders[0].items.as_ref().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].quantity, 2);

    let product = items[0].product.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(product.name, "Foundry Mug");

    let relation_loaded_user = users[0].clone();
    let updated_user = relation_loaded_user
        .update()
        .set(User::ACTIVE, false)
        .save(app)
        .await
        .unwrap();
    assert!(!updated_user.active);
    assert!(!updated_user.merchants.is_loaded());
    assert!(!updated_user.merchant_count.is_loaded());
}

#[tokio::test]
async fn relation_eager_loading_chunks_large_key_sets_and_preserves_duplicates() {
    let Some(database) = test_database().await else {
        return;
    };
    let _guard = database_lock().lock().await;
    reset_schema(&database).await;

    execute_batch(
        &database,
        &[
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active) SELECT i, 'chunk-user-' || i || '@example.com', true FROM generate_series(1, 1005) AS i"
            ),
            &format!(
                "INSERT INTO {MERCHANTS_TABLE} (id, user_id, name, status) SELECT i, i, 'Merchant ' || i, 'active' FROM generate_series(1, 1005) AS i"
            ),
            &format!(
                "INSERT INTO {TAGS_TABLE} (id, user_id, name) SELECT i, i, 'Tag ' || i FROM generate_series(1, 1005) AS i"
            ),
            &format!(
                "INSERT INTO {MERCHANT_TAGS_TABLE} (merchant_id, tag_id, role) SELECT i, i, 'primary' FROM generate_series(1, 1005) AS i"
            ),
        ],
    )
    .await;

    let empty = User::query()
        .where_(User::ID.eq(-1_i64))
        .with(User::merchants())
        .get(&database)
        .await
        .unwrap();
    assert!(empty.is_empty());

    let users = User::query()
        .order_by(User::ID.asc())
        .with(User::merchants())
        .with_aggregate(User::active_merchant_count())
        .get(&database)
        .await
        .unwrap();
    assert_eq!(users.len(), 1005);
    assert_eq!(users[0].merchants.as_ref().unwrap().len(), 1);
    assert_eq!(users[1004].merchants.as_ref().unwrap()[0].id, 1005);
    assert_eq!(users[1004].merchant_count.as_ref(), Some(&1));

    let duplicated = Collection::from_vec(vec![users[0].clone(), users[0].clone()])
        .load(User::merchants(), &database)
        .await
        .unwrap();
    assert_eq!(duplicated[0].merchants.as_ref().unwrap().len(), 1);
    assert_eq!(duplicated[1].merchants.as_ref().unwrap().len(), 1);

    let merchants = Merchant::query()
        .order_by(Merchant::ID.asc())
        .with_many_to_many(Merchant::tags())
        .with_aggregate(Merchant::tags_count())
        .get(&database)
        .await
        .unwrap();
    assert_eq!(merchants.len(), 1005);
    assert_eq!(merchants[0].tags.as_ref().unwrap()[0].id, 1);
    assert_eq!(merchants[1004].tags.as_ref().unwrap()[0].id, 1005);
    assert_eq!(merchants[1004].tag_count.as_ref(), Some(&1));
}

#[tokio::test]
async fn model_stream_supports_relation_trees_where_has_and_aggregates() {
    let Some(database) = test_database().await else {
        return;
    };
    let _guard = database_lock().lock().await;
    reset_schema(&database).await;

    execute_batch(
        &database,
        &[
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active) VALUES (1, 'stream-owner@example.com', true)"
            ),
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active) VALUES (2, 'stream-second@example.com', true)"
            ),
            &format!(
                "INSERT INTO {MERCHANTS_TABLE} (id, user_id, name, status) VALUES (10, 1, 'Foundry Store', 'active')"
            ),
            &format!(
                "INSERT INTO {MERCHANTS_TABLE} (id, user_id, name, status) VALUES (11, 1, 'Archived Store', 'suspended')"
            ),
            &format!(
                "INSERT INTO {MERCHANTS_TABLE} (id, user_id, name, status) VALUES (12, 2, 'Second Store', 'active')"
            ),
            &format!("INSERT INTO {ORDERS_TABLE} (id, merchant_id, total) VALUES (20, 10, 2500)"),
            &format!("INSERT INTO {ORDERS_TABLE} (id, merchant_id, total) VALUES (21, 12, 1500)"),
            &format!("INSERT INTO {PRODUCTS_TABLE} (id, name) VALUES (30, 'Foundry Mug')"),
            &format!("INSERT INTO {PRODUCTS_TABLE} (id, name) VALUES (31, 'Foundry Pen')"),
            &format!(
                "INSERT INTO {ORDER_ITEMS_TABLE} (id, order_id, product_id, quantity) VALUES (40, 20, 30, 2)"
            ),
            &format!(
                "INSERT INTO {ORDER_ITEMS_TABLE} (id, order_id, product_id, quantity) VALUES (41, 21, 31, 1)"
            ),
        ],
    )
    .await;

    let users = User::query()
        .with_aggregate(User::active_merchant_count())
        .with(
            User::merchants()
                .with_aggregate(Merchant::orders_total())
                .with(Merchant::orders().with(Order::items().with(OrderItem::product()))),
        )
        .where_has(User::merchants(), |query| {
            query.where_(Merchant::STATUS.eq(MerchantStatus::Active))
        })
        .order_by(User::ID.asc())
        .with_label("relation tree stream")
        .with_stream_batch_size(1)
        .stream(&database)
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();

    assert_eq!(
        users.iter().map(|user| user.id).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(users[0].merchant_count.as_ref(), Some(&1));
    assert_eq!(users[1].merchant_count.as_ref(), Some(&1));

    let first_merchant = &users[0].merchants.as_ref().unwrap()[0];
    assert_eq!(first_merchant.name, "Foundry Store");
    assert_eq!(first_merchant.order_total.as_ref(), Some(&Some(2500)));
    let first_product = users[0].merchants.as_ref().unwrap()[0]
        .orders
        .as_ref()
        .unwrap()[0]
        .items
        .as_ref()
        .unwrap()[0]
        .product
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(first_product.name, "Foundry Mug");

    let second_merchant = &users[1].merchants.as_ref().unwrap()[0];
    assert_eq!(second_merchant.name, "Second Store");
    assert_eq!(second_merchant.order_total.as_ref(), Some(&Some(1500)));
}

#[tokio::test]
async fn advanced_projection_queries_support_cte_case_json_union_and_numeric_aggregates() {
    let Some(database) = test_database().await else {
        return;
    };
    let _guard = database_lock().lock().await;
    reset_schema(&database).await;

    execute_batch(
        &database,
        &[
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active, metadata) VALUES (1, 'owner@example.com', true, '{{\"theme\":\"amber\"}}'::jsonb)"
            ),
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active, metadata) VALUES (2, 'guest@example.com', false, '{{\"theme\":\"slate\"}}'::jsonb)"
            ),
            &format!(
                "INSERT INTO {TAGS_TABLE} (id, user_id, name) VALUES (100, 1, 'featured')"
            ),
        ],
    )
    .await;

    Query::insert_into(PAYMENTS_TABLE)
        .value("id", 1_i64)
        .value("merchant_id", 10_i64)
        .value("amount", Numeric::new("10.50").unwrap())
        .value("metadata", serde_json::json!({"vip": true}))
        .execute(&database)
        .await
        .unwrap();
    Query::insert_into(PAYMENTS_TABLE)
        .value("id", 2_i64)
        .value("merchant_id", 10_i64)
        .value("amount", Numeric::new("20.25").unwrap())
        .value("metadata", serde_json::json!({"vip": false}))
        .execute(&database)
        .await
        .unwrap();
    Query::insert_into(PAYMENTS_TABLE)
        .value("id", 3_i64)
        .value("merchant_id", 11_i64)
        .value("amount", Numeric::new("30.00").unwrap())
        .value("metadata", serde_json::json!({"vip": true}))
        .execute(&database)
        .await
        .unwrap();

    let active_users = Query::table(USERS_TABLE)
        .select_expr(
            ColumnRef::new(USERS_TABLE, "email"),
            UserPreferenceRow::EMAIL.alias(),
        )
        .select_expr(
            Case::when(
                Condition::compare(
                    Expr::column(ColumnRef::new(USERS_TABLE, "active")),
                    ComparisonOp::Eq,
                    Expr::value(true),
                ),
                Expr::value("active"),
            )
            .else_(Expr::value("inactive")),
            UserPreferenceRow::STATUS_LABEL.alias(),
        )
        .select_expr(
            Expr::column(ColumnRef::new(USERS_TABLE, "metadata").typed(DbType::Json))
                .json()
                .key("theme")
                .as_text(),
            UserPreferenceRow::THEME.alias(),
        );

    let preference_rows = ProjectionQuery::<UserPreferenceRow>::table("active_users")
        .with_cte(Cte::new("active_users", active_users))
        .select_source(UserPreferenceRow::EMAIL, "active_users")
        .select_source(UserPreferenceRow::STATUS_LABEL, "active_users")
        .select_source(UserPreferenceRow::THEME, "active_users")
        .order_by(OrderBy::asc(UserPreferenceRow::EMAIL.alias()))
        .get(&database)
        .await
        .unwrap();

    assert_eq!(preference_rows.len(), 2);
    assert_eq!(preference_rows[0].theme, "slate");
    assert_eq!(preference_rows[1].status_label, "active");

    let combined_labels = ProjectionQuery::<CombinedLabelRow>::table(USERS_TABLE)
        .select_field(
            CombinedLabelRow::LABEL,
            ColumnRef::new(USERS_TABLE, "email"),
        )
        .select_field(CombinedLabelRow::KIND, Expr::value("user"))
        .union_all(
            ProjectionQuery::<CombinedLabelRow>::table(TAGS_TABLE)
                .select_field(CombinedLabelRow::LABEL, ColumnRef::new(TAGS_TABLE, "name"))
                .select_field(CombinedLabelRow::KIND, Expr::value("tag")),
        )
        .order_by(OrderBy::asc(CombinedLabelRow::LABEL.alias()))
        .get(&database)
        .await
        .unwrap();
    assert_eq!(combined_labels.len(), 3);
    assert_eq!(combined_labels[0].kind, "tag");

    let user_tag_rows = Query::table(USERS_TABLE)
        .left_join(
            TAGS_TABLE,
            Condition::compare(
                Expr::column(ColumnRef::new(TAGS_TABLE, "user_id")),
                ComparisonOp::Eq,
                Expr::column(ColumnRef::new(USERS_TABLE, "id")),
            ),
        )
        .select_expr(ColumnRef::new(USERS_TABLE, "email"), "email")
        .select_expr(ColumnRef::new(TAGS_TABLE, "name"), "tag_name")
        .order_by(OrderBy::asc(ColumnRef::new(USERS_TABLE, "id")))
        .get(&database)
        .await
        .unwrap();
    assert_eq!(user_tag_rows.len(), 2);
    assert_eq!(
        user_tag_rows[0].decode::<String>("email").unwrap(),
        "owner@example.com"
    );
    assert_eq!(
        user_tag_rows[0]
            .decode::<Option<String>>("tag_name")
            .unwrap(),
        Some("featured".to_string())
    );
    assert_eq!(
        user_tag_rows[1]
            .decode::<Option<String>>("tag_name")
            .unwrap(),
        None
    );

    let cross_join_total = Query::table(USERS_TABLE)
        .cross_join(TAGS_TABLE)
        .count(&database)
        .await
        .unwrap();
    assert_eq!(cross_join_total, 2);

    let payment_query = Query::table(PAYMENTS_TABLE).where_(
        Expr::column(ColumnRef::new(PAYMENTS_TABLE, "metadata").typed(DbType::Json))
            .json()
            .has_key("vip"),
    );
    let distinct_merchants = payment_query
        .count_distinct(&database, ColumnRef::new(PAYMENTS_TABLE, "merchant_id"))
        .await
        .unwrap();
    let total_amount = payment_query
        .sum::<_, Numeric>(
            &database,
            ColumnRef::new(PAYMENTS_TABLE, "amount").typed(DbType::Numeric),
        )
        .await
        .unwrap();
    let average_amount = payment_query
        .avg::<_, Numeric>(
            &database,
            ColumnRef::new(PAYMENTS_TABLE, "amount").typed(DbType::Numeric),
        )
        .await
        .unwrap();
    let min_amount = payment_query
        .min::<_, Numeric>(
            &database,
            ColumnRef::new(PAYMENTS_TABLE, "amount").typed(DbType::Numeric),
        )
        .await
        .unwrap();
    let max_amount = payment_query
        .max::<_, Numeric>(
            &database,
            ColumnRef::new(PAYMENTS_TABLE, "amount").typed(DbType::Numeric),
        )
        .await
        .unwrap();

    assert_eq!(distinct_merchants, 2);
    assert_eq!(total_amount.unwrap().as_str(), "60.75");
    assert!(average_amount.unwrap().as_str().starts_with("20.25"));
    assert!(min_amount.unwrap().as_str().starts_with("10.5"));
    assert!(max_amount.unwrap().as_str().starts_with("30"));
}

#[tokio::test]
async fn many_to_many_relations_load_pivot_data_and_aggregates() {
    let Some(database) = test_database().await else {
        return;
    };
    let _guard = database_lock().lock().await;
    reset_schema(&database).await;

    execute_batch(
        &database,
        &[
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active) VALUES (1, 'owner@example.com', true)"
            ),
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active) VALUES (2, 'creator@example.com', true)"
            ),
            &format!(
                "INSERT INTO {MERCHANTS_TABLE} (id, user_id, name, status) VALUES (10, 1, 'Foundry Store', 'active')"
            ),
            &format!(
                "INSERT INTO {TAGS_TABLE} (id, user_id, name) VALUES (100, 2, 'featured')"
            ),
            &format!(
                "INSERT INTO {TAGS_TABLE} (id, user_id, name) VALUES (101, 2, 'seasonal')"
            ),
            &format!(
                "INSERT INTO {MERCHANT_TAGS_TABLE} (merchant_id, tag_id, role) VALUES (10, 100, 'primary')"
            ),
            &format!(
                "INSERT INTO {MERCHANT_TAGS_TABLE} (merchant_id, tag_id, role) VALUES (10, 101, 'secondary')"
            ),
        ],
    )
    .await;

    let merchants = Merchant::query()
        .with_aggregate(Merchant::tags_count())
        .with_many_to_many(Merchant::tags_with_pivot().with(Tag::creator()))
        .where_(Merchant::STATUS.eq(MerchantStatus::Active))
        .get(&database)
        .await
        .unwrap();

    assert_eq!(merchants.len(), 1);
    let merchant = &merchants[0];
    assert_eq!(merchant.tag_count.as_ref(), Some(&2));
    let tags = merchant.tags.as_ref().unwrap();
    assert_eq!(tags.len(), 2);
    assert_eq!(tags[0].link.as_ref().unwrap().role, "primary");
    assert_eq!(
        tags[0].creator.as_ref().unwrap().as_ref().unwrap().email,
        "creator@example.com"
    );
}

#[tokio::test]
async fn model_stream_supports_many_to_many_pivot_hydration_inside_transactions() {
    let Some(database) = test_database().await else {
        return;
    };
    let _guard = database_lock().lock().await;
    reset_schema(&database).await;

    execute_batch(
        &database,
        &[
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active) VALUES (1, 'owner@example.com', true)"
            ),
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active) VALUES (2, 'creator@example.com', true)"
            ),
            &format!(
                "INSERT INTO {MERCHANTS_TABLE} (id, user_id, name, status) VALUES (10, 1, 'Foundry Store', 'active')"
            ),
            &format!(
                "INSERT INTO {MERCHANTS_TABLE} (id, user_id, name, status) VALUES (11, 1, 'Foundry Supplies', 'active')"
            ),
            &format!(
                "INSERT INTO {TAGS_TABLE} (id, user_id, name) VALUES (100, 2, 'featured')"
            ),
            &format!(
                "INSERT INTO {TAGS_TABLE} (id, user_id, name) VALUES (101, 2, 'seasonal')"
            ),
            &format!(
                "INSERT INTO {MERCHANT_TAGS_TABLE} (merchant_id, tag_id, role) VALUES (10, 100, 'primary')"
            ),
            &format!(
                "INSERT INTO {MERCHANT_TAGS_TABLE} (merchant_id, tag_id, role) VALUES (10, 101, 'secondary')"
            ),
            &format!(
                "INSERT INTO {MERCHANT_TAGS_TABLE} (merchant_id, tag_id, role) VALUES (11, 100, 'primary')"
            ),
        ],
    )
    .await;

    let transaction = database.begin().await.unwrap();
    let merchants = Merchant::query()
        .with_aggregate(Merchant::tags_count())
        .with_many_to_many(Merchant::tags_with_pivot().with(Tag::creator()))
        .order_by(Merchant::ID.asc())
        .with_label("many-to-many transaction stream")
        .with_stream_batch_size(0)
        .stream(&transaction)
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();

    assert_eq!(
        merchants
            .iter()
            .map(|merchant| merchant.id)
            .collect::<Vec<_>>(),
        vec![10, 11]
    );
    assert_eq!(merchants[0].tag_count.as_ref(), Some(&2));
    assert_eq!(merchants[1].tag_count.as_ref(), Some(&1));
    assert_eq!(
        merchants[0].tags.as_ref().unwrap()[0]
            .link
            .as_ref()
            .unwrap()
            .role,
        "primary"
    );
    assert_eq!(
        merchants[0].tags.as_ref().unwrap()[0]
            .creator
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .email,
        "creator@example.com"
    );
}

#[tokio::test]
async fn typed_runtime_supports_production_postgres_values_and_custom_adapters() {
    let Some(database) = test_database().await else {
        return;
    };
    let _guard = database_lock().lock().await;

    execute_batch(
        &database,
        &[
            "DROP DOMAIN IF EXISTS foundry_test_email",
            "DROP TYPE IF EXISTS foundry_test_mood",
            "CREATE TYPE foundry_test_mood AS ENUM ('happy', 'sad')",
            "CREATE DOMAIN foundry_test_email AS TEXT",
        ],
    )
    .await;

    let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let model_id = ModelId::<SafeUuidUser>::from_uuid(uuid);
    let timestamp_tz = DateTime::parse("2024-01-02T03:04:05Z").unwrap();
    let date = Date::parse("2024-01-02").unwrap();
    let timestamp = LocalDateTime::parse("2024-01-02T03:04:05").unwrap();
    let time = Time::parse("06:07:08").unwrap();
    let numeric = Numeric::new("42.75").unwrap();

    let scalar_record = database
        .raw_query(
            "SELECT $1::smallint AS int16_value, $2::integer AS int32_value, $3::bigint AS int64_value, $4::real AS float32_value, $5::double precision AS float64_value, $6::numeric AS numeric_value, $7::text AS text_value, $8::jsonb AS json_value, $9::uuid AS uuid_value, $10::timestamptz AS timestamptz_value, $11::timestamp AS timestamp_value, $12::date AS date_value, $13::time AS time_value, $14::bytea AS bytea_value",
            &[
                7_i16.into(),
                8_i32.into(),
                9_i64.into(),
                1.5_f32.into(),
                2.5_f64.into(),
                numeric.clone().into(),
                "foundry".into(),
                serde_json::json!({"theme":"amber"}).into(),
                model_id.into(),
                timestamp_tz.into(),
                timestamp.into(),
                date.into(),
                time.into(),
                vec![1_u8, 2, 3].into(),
            ],
        )
        .await
        .unwrap()
        .remove(0);

    assert_eq!(scalar_record.decode::<i16>("int16_value").unwrap(), 7);
    assert_eq!(scalar_record.decode::<i32>("int32_value").unwrap(), 8);
    assert_eq!(scalar_record.decode::<i64>("int64_value").unwrap(), 9);
    assert!((scalar_record.decode::<f32>("float32_value").unwrap() - 1.5).abs() < f32::EPSILON);
    assert!((scalar_record.decode::<f64>("float64_value").unwrap() - 2.5).abs() < f64::EPSILON);
    assert_eq!(
        scalar_record
            .decode::<Numeric>("numeric_value")
            .unwrap()
            .as_str(),
        "42.75"
    );
    assert_eq!(
        scalar_record.decode::<String>("text_value").unwrap(),
        "foundry".to_string()
    );
    assert_eq!(
        scalar_record
            .decode::<serde_json::Value>("json_value")
            .unwrap()["theme"],
        "amber"
    );
    assert_eq!(scalar_record.decode::<Uuid>("uuid_value").unwrap(), uuid);
    assert_eq!(
        scalar_record
            .decode::<ModelId<SafeUuidUser>>("uuid_value")
            .unwrap(),
        model_id
    );
    assert_eq!(
        scalar_record
            .decode::<DateTime>("timestamptz_value")
            .unwrap(),
        timestamp_tz
    );
    assert_eq!(
        scalar_record
            .decode::<LocalDateTime>("timestamp_value")
            .unwrap(),
        timestamp
    );
    assert_eq!(scalar_record.decode::<Date>("date_value").unwrap(), date);
    assert_eq!(scalar_record.decode::<Time>("time_value").unwrap(), time);
    assert_eq!(
        scalar_record.decode::<Vec<u8>>("bytea_value").unwrap(),
        vec![1, 2, 3]
    );

    let array_record = database
        .raw_query(
            "SELECT $1::smallint[] AS int16_values, $2::integer[] AS int32_values, $3::bigint[] AS int64_values, $4::boolean[] AS bool_values, $5::real[] AS float32_values, $6::double precision[] AS float64_values, $7::numeric[] AS numeric_values, $8::text[] AS text_values, $9::jsonb[] AS json_values, $10::uuid[] AS uuid_values, $11::timestamptz[] AS timestamptz_values, $12::timestamp[] AS timestamp_values, $13::date[] AS date_values, $14::time[] AS time_values, $15::bytea[] AS bytea_values",
            &[
                vec![1_i16, 2_i16].into(),
                vec![3_i32, 4_i32].into(),
                vec![5_i64, 6_i64].into(),
                vec![true, false].into(),
                vec![1.25_f32, 2.5_f32].into(),
                vec![3.5_f64, 4.75_f64].into(),
                vec![Numeric::new("1.10").unwrap(), Numeric::new("2.20").unwrap()].into(),
                vec!["alpha".to_string(), "beta".to_string()].into(),
                vec![serde_json::json!({"rank":1}), serde_json::json!({"rank":2})].into(),
                vec![model_id].into(),
                vec![timestamp_tz].into(),
                vec![timestamp].into(),
                vec![date].into(),
                vec![time].into(),
                vec![vec![9_u8, 8, 7]].into(),
            ],
        )
        .await
        .unwrap()
        .remove(0);

    assert_eq!(
        array_record.decode::<Vec<i16>>("int16_values").unwrap(),
        vec![1, 2]
    );
    assert_eq!(
        array_record.decode::<Vec<i32>>("int32_values").unwrap(),
        vec![3, 4]
    );
    assert_eq!(
        array_record.decode::<Vec<i64>>("int64_values").unwrap(),
        vec![5, 6]
    );
    assert_eq!(
        array_record.decode::<Vec<bool>>("bool_values").unwrap(),
        vec![true, false]
    );
    assert_eq!(
        array_record.decode::<Vec<f32>>("float32_values").unwrap(),
        vec![1.25, 2.5]
    );
    assert_eq!(
        array_record.decode::<Vec<f64>>("float64_values").unwrap(),
        vec![3.5, 4.75]
    );
    assert_eq!(
        array_record
            .decode::<Vec<Numeric>>("numeric_values")
            .unwrap()
            .iter()
            .map(Numeric::as_str)
            .collect::<Vec<_>>(),
        vec!["1.10", "2.20"]
    );
    assert_eq!(
        array_record.decode::<Vec<String>>("text_values").unwrap(),
        vec!["alpha".to_string(), "beta".to_string()]
    );
    assert_eq!(
        array_record.decode::<Vec<Uuid>>("uuid_values").unwrap(),
        vec![uuid]
    );
    assert_eq!(
        array_record
            .decode::<Vec<ModelId<SafeUuidUser>>>("uuid_values")
            .unwrap(),
        vec![model_id]
    );
    assert_eq!(
        array_record
            .decode::<Vec<DateTime>>("timestamptz_values")
            .unwrap(),
        vec![timestamp_tz]
    );
    assert_eq!(
        array_record
            .decode::<Vec<LocalDateTime>>("timestamp_values")
            .unwrap(),
        vec![timestamp]
    );
    assert_eq!(
        array_record.decode::<Vec<Date>>("date_values").unwrap(),
        vec![date]
    );
    assert_eq!(
        array_record.decode::<Vec<Time>>("time_values").unwrap(),
        vec![time]
    );
    assert_eq!(
        array_record.decode::<Vec<Vec<u8>>>("bytea_values").unwrap(),
        vec![vec![9, 8, 7]]
    );

    let text_alias_record = database
        .raw_query(
            "SELECT ARRAY['alpha', 'beta']::TEXT[] AS text_alias_values",
            &[],
        )
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        text_alias_record
            .decode::<Vec<String>>("text_alias_values")
            .unwrap(),
        vec!["alpha".to_string(), "beta".to_string()]
    );

    let unsupported = database
        .raw_query("SELECT 'happy'::foundry_test_mood AS mood", &[])
        .await
        .unwrap_err();
    let unsupported_message = format!("{unsupported:?}");
    assert!(unsupported_message.contains("unsupported postgres type `foundry_test_mood`"));
    assert!(unsupported_message.contains("normalized lookup `foundry_test_mood`"));
    assert!(unsupported_message.contains("column `mood`"));

    database
        .register_type_adapter("foundry_test_mood", DbType::Text)
        .unwrap();
    database
        .register_type_adapter("foundry_test_email", DbType::Text)
        .unwrap();

    let custom_record = database
        .raw_query(
            "SELECT 'happy'::foundry_test_mood AS mood, 'owner@example.com'::foundry_test_email AS email",
            &[],
        )
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        custom_record.decode::<String>("mood").unwrap(),
        "happy".to_string()
    );
    assert_eq!(
        custom_record.decode::<String>("email").unwrap(),
        "owner@example.com".to_string()
    );

    execute_batch(
        &database,
        &[
            "DROP DOMAIN IF EXISTS foundry_test_email",
            "DROP TYPE IF EXISTS foundry_test_mood",
        ],
    )
    .await;
}

#[tokio::test]
async fn locking_streaming_timeout_and_debug_surfaces_work() {
    let Some(database) = test_database().await else {
        return;
    };
    let _guard = database_lock().lock().await;
    reset_schema(&database).await;

    execute_batch(
        &database,
        &[
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active) VALUES (1, 'stream-one@example.com', true)"
            ),
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active) VALUES (2, 'stream-two@example.com', false)"
            ),
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active) VALUES (3, 'stream-three@example.com', true)"
            ),
        ],
    )
    .await;

    let generic_ids = Query::table(USERS_TABLE)
        .select(["id"])
        .order_by(OrderBy::asc("id"))
        .with_label("generic stream")
        .stream(&database)
        .unwrap()
        .map_ok(|record| record.decode::<i64>("id").unwrap())
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(generic_ids, vec![1, 2, 3]);

    let user_ids = User::query()
        .order_by(User::ID.asc())
        .with_label("model stream")
        .stream(&database)
        .unwrap()
        .map_ok(|user| user.id)
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(user_ids, vec![1, 2, 3]);

    let label_rows = ProjectionQuery::<CombinedLabelRow>::table(USERS_TABLE)
        .select_field(
            CombinedLabelRow::LABEL,
            ColumnRef::new(USERS_TABLE, "email"),
        )
        .select_field(CombinedLabelRow::KIND, Expr::value("user"))
        .order_by(OrderBy::asc(CombinedLabelRow::LABEL.alias()))
        .with_label("projection stream")
        .stream(&database)
        .unwrap()
        .map_ok(|row| row.label)
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(
        label_rows,
        vec![
            "stream-one@example.com".to_string(),
            "stream-three@example.com".to_string(),
            "stream-two@example.com".to_string(),
        ]
    );

    let mut raw_stream = database.raw_stream(
        "SELECT slow_values.value FROM generate_series(1, 3) AS slow_values(value), LATERAL (SELECT pg_sleep(0.02)) AS pause",
        &[],
        QueryExecutionOptions::default().with_label("slow raw stream"),
    );
    let first_raw_row = tokio::time::timeout(Duration::from_millis(40), raw_stream.try_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first_raw_row.decode::<i64>("value").unwrap(), 1);

    let explain_lines = User::query()
        .where_(User::ID.eq(1_i64))
        .with_label("user explain")
        .explain(&database)
        .await
        .unwrap();
    assert!(!explain_lines.is_empty());

    let transaction = database.begin().await.unwrap();
    transaction
        .raw_query(
            &format!("SELECT id FROM {USERS_TABLE} WHERE id = $1 FOR UPDATE"),
            &[1_i64.into()],
        )
        .await
        .unwrap();

    let skip_locked = Query::table(USERS_TABLE)
        .select(["id"])
        .where_eq("id", 1_i64)
        .for_update()
        .skip_locked()
        .first(&database)
        .await
        .unwrap();
    assert!(skip_locked.is_none());

    let lock_error = Query::table(USERS_TABLE)
        .select(["id"])
        .where_eq("id", 1_i64)
        .for_update()
        .nowait()
        .first(&database)
        .await
        .unwrap_err();
    assert!(format!("{lock_error:?}")
        .to_ascii_lowercase()
        .contains("lock"));

    transaction.rollback().await.unwrap();

    let timeout_error = database
        .raw_query_with(
            "SELECT pg_sleep(0.05)",
            &[],
            QueryExecutionOptions::default()
                .with_timeout(Duration::from_millis(5))
                .with_label("sleep probe"),
        )
        .await
        .unwrap_err();
    let timeout_message = format!("{timeout_error:?}");
    assert!(timeout_message.contains("timed out"));
    assert!(timeout_message.contains("sleep probe"));

    let timeout_tx = database.begin().await.unwrap();
    let tx_timeout_error = timeout_tx
        .raw_query_with(
            "SELECT pg_sleep(0.05)",
            &[],
            QueryExecutionOptions::default()
                .with_timeout(Duration::from_millis(5))
                .with_label("tx sleep probe"),
        )
        .await
        .unwrap_err();
    let tx_timeout_message = format!("{tx_timeout_error:?}");
    assert!(tx_timeout_message.contains("timed out"));
    assert!(tx_timeout_message.contains("tx sleep probe"));
    timeout_tx.rollback().await.unwrap();

    execute_batch(&database, &["DROP TYPE IF EXISTS foundry_stream_mood"]).await;
    database
        .raw_execute("CREATE TYPE foundry_stream_mood AS ENUM ('happy')", &[])
        .await
        .unwrap();
    let mut unsupported_stream = database.raw_stream(
        "SELECT 'happy'::foundry_stream_mood AS mood",
        &[],
        QueryExecutionOptions::default().with_label("unsupported stream"),
    );
    let unsupported_stream_error = unsupported_stream.try_next().await.unwrap_err();
    let unsupported_stream_message = format!("{unsupported_stream_error:?}");
    assert!(unsupported_stream_message.contains("unsupported postgres type `foundry_stream_mood`"));
    assert!(unsupported_stream_message.contains("normalized lookup `foundry_stream_mood`"));
    assert!(unsupported_stream_message.contains("unsupported stream"));
    execute_batch(&database, &["DROP TYPE IF EXISTS foundry_stream_mood"]).await;
}

#[tokio::test]
async fn dropping_raw_stream_cancels_in_flight_producer_and_releases_pool_slot() {
    let Some(url) = postgres_url() else {
        return;
    };
    let database = DatabaseManager::from_config(&DatabaseConfig {
        url,
        min_connections: 0,
        max_connections: 1,
        acquire_timeout_ms: 200,
        ..DatabaseConfig::default()
    })
    .await
    .unwrap();

    let stream = database.raw_stream(
        "SELECT pg_sleep(5)::text AS slept",
        &[],
        QueryExecutionOptions::default().with_label("dropped slow stream"),
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(stream);

    let rows = tokio::time::timeout(
        Duration::from_secs(1),
        database.raw_query("SELECT 1::bigint AS value", &[]),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(rows[0].decode::<i64>("value").unwrap(), 1);
}

#[tokio::test]
async fn typed_runtime_hydrates_published_countries_char_columns_as_strings() {
    let Some(database) = test_database().await else {
        return;
    };
    let _guard = database_lock().lock().await;

    execute_batch(
        &database,
        &[
            &format!("DROP TABLE IF EXISTS {COUNTRIES_RUNTIME_TABLE}"),
            &format!(
                "CREATE TABLE {COUNTRIES_RUNTIME_TABLE} (iso2 CHAR(2) PRIMARY KEY, iso3 CHAR(3) NOT NULL, name TEXT NOT NULL)"
            ),
            &format!(
                "INSERT INTO {COUNTRIES_RUNTIME_TABLE} (iso2, iso3, name) VALUES ('MY', 'MYS', 'Malaysia')"
            ),
        ],
    )
    .await;

    let record = database
        .raw_query(
            &format!(
                "SELECT iso2, iso3, ARRAY[iso2, 'US'::CHAR(2)]::CHAR(2)[] AS iso2_values, ARRAY[iso3, 'USA'::CHAR(3)]::CHAR(3)[] AS iso3_values FROM {COUNTRIES_RUNTIME_TABLE} WHERE iso2 = 'MY'"
            ),
            &[],
        )
        .await
        .unwrap()
        .remove(0);

    assert_eq!(record.decode::<String>("iso2").unwrap(), "MY".to_string());
    assert_eq!(record.decode::<String>("iso3").unwrap(), "MYS".to_string());
    assert_eq!(
        record.decode::<Vec<String>>("iso2_values").unwrap(),
        vec!["MY".to_string(), "US".to_string()]
    );
    assert_eq!(
        record.decode::<Vec<String>>("iso3_values").unwrap(),
        vec!["MYS".to_string(), "USA".to_string()]
    );

    execute_batch(
        &database,
        &[&format!("DROP TABLE IF EXISTS {COUNTRIES_RUNTIME_TABLE}")],
    )
    .await;
}

#[tokio::test]
async fn advanced_mutation_queries_execute_against_postgres() {
    let Some(runtime) = test_app_runtime().await else {
        return;
    };
    let database = runtime.database.as_ref();
    let app = &runtime.app;
    let _guard = database_lock().lock().await;
    reset_schema(database).await;

    execute_batch(
        database,
        &[
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active) VALUES (1, 'owner@example.com', true)"
            ),
            &format!(
                "INSERT INTO {USERS_TABLE} (id, email, active) VALUES (2, 'inactive@example.com', false)"
            ),
            &format!(
                "INSERT INTO {MERCHANTS_TABLE} (id, user_id, name, status) VALUES (10, 1, 'Foundry Store', 'active')"
            ),
            &format!(
                "INSERT INTO {MERCHANTS_TABLE} (id, user_id, name, status) VALUES (11, 2, 'Dormant Store', 'active')"
            ),
            "DROP TABLE IF EXISTS foundry_test_user_archive",
            "CREATE TEMP TABLE foundry_test_user_archive (id BIGINT PRIMARY KEY, email TEXT NOT NULL)",
        ],
    )
    .await;

    let archived = Query::insert_select_into(
        "foundry_test_user_archive",
        Query::table(USERS_TABLE)
            .select(["id", "email"])
            .where_eq("active", false),
    )
    .execute(database)
    .await
    .unwrap();
    assert_eq!(archived, 1);

    let archive_rows = Query::table("foundry_test_user_archive")
        .select(["id", "email"])
        .get(database)
        .await
        .unwrap();
    assert_eq!(archive_rows.len(), 1);
    assert_eq!(
        archive_rows[0].decode::<String>("email").unwrap(),
        "inactive@example.com".to_string()
    );

    let suspended = Merchant::update()
        .set(Merchant::STATUS, MerchantStatus::Suspended)
        .from(USERS_TABLE)
        .where_(Condition::and([
            Condition::compare(
                Expr::column(ColumnRef::new(MERCHANTS_TABLE, "user_id")),
                ComparisonOp::Eq,
                Expr::column(ColumnRef::new(USERS_TABLE, "id")),
            ),
            Condition::compare(
                Expr::column(ColumnRef::new(USERS_TABLE, "active")),
                ComparisonOp::Eq,
                Expr::value(false),
            ),
        ]))
        .save(app)
        .await
        .unwrap();
    assert_eq!(suspended.id, 11);
    assert_eq!(suspended.status, MerchantStatus::Suspended);

    let renamed = Query::update_table(MERCHANTS_TABLE)
        .set_expr("name", ColumnRef::new(USERS_TABLE, "email"))
        .from(USERS_TABLE)
        .where_(Condition::and([
            Condition::compare(
                Expr::column(ColumnRef::new(MERCHANTS_TABLE, "user_id")),
                ComparisonOp::Eq,
                Expr::column(ColumnRef::new(USERS_TABLE, "id")),
            ),
            Condition::compare(
                Expr::column(ColumnRef::new(USERS_TABLE, "id")),
                ComparisonOp::Eq,
                Expr::value(1_i64),
            ),
        ]))
        .returning(["name"])
        .get(database)
        .await
        .unwrap();
    assert_eq!(
        renamed[0].decode::<String>("name").unwrap(),
        "owner@example.com".to_string()
    );

    let deleted = Merchant::delete()
        .using(USERS_TABLE)
        .where_(Condition::and([
            Condition::compare(
                Expr::column(ColumnRef::new(MERCHANTS_TABLE, "user_id")),
                ComparisonOp::Eq,
                Expr::column(ColumnRef::new(USERS_TABLE, "id")),
            ),
            Condition::compare(
                Expr::column(ColumnRef::new(USERS_TABLE, "active")),
                ComparisonOp::Eq,
                Expr::value(false),
            ),
        ]))
        .execute(app)
        .await
        .unwrap();
    assert_eq!(deleted, 1);

    let remaining_merchants = Merchant::query()
        .order_by(Merchant::ID.asc())
        .get(database)
        .await
        .unwrap();
    assert_eq!(remaining_merchants.len(), 1);
    assert_eq!(remaining_merchants[0].name, "owner@example.com");
}
