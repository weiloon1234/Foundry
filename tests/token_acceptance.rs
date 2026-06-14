use std::fs;
use std::sync::Arc;
use std::sync::OnceLock;

use async_trait::async_trait;
use foundry::kernel::cli::CliKernel;
use foundry::prelude::*;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tempfile::TempDir;
use tokio::sync::{Mutex, MutexGuard};

const PAT_TABLE: &str = "personal_access_tokens";
const RESET_TABLE: &str = "password_reset_tokens";
const MFA_TABLE: &str = "auth_mfa_totp_factors";
const TOTP_PERIOD_SECONDS: i64 = 30;
const TOTP_DIGITS: u32 = 6;

type HmacSha256 = Hmac<Sha256>;

fn postgres_url() -> Option<String> {
    std::env::var("FOUNDRY_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

async fn token_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().await
}

struct TokenTestRuntime {
    _dir: TempDir,
    app: AppContext,
    database: Arc<DatabaseManager>,
}

impl TokenTestRuntime {
    async fn new() -> Option<Self> {
        Self::new_with_config("").await
    }

    async fn new_with_config(extra_config: &str) -> Option<Self> {
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
            .build_cli_kernel()
            .await
            .ok()?;
        let app = kernel.app().clone();
        let database = app.database().ok()?;

        reset_personal_access_tokens(database.as_ref()).await;
        reset_password_reset_tokens(database.as_ref()).await;
        reset_mfa_totp_factors(database.as_ref()).await;

        Some(Self {
            _dir: dir,
            app,
            database,
        })
    }

    async fn cleanup(&self) {
        let _ = self
            .database
            .raw_execute(&format!("DROP TABLE IF EXISTS {PAT_TABLE}"), &[])
            .await;
        let _ = self
            .database
            .raw_execute(&format!("DROP TABLE IF EXISTS {RESET_TABLE}"), &[])
            .await;
        let _ = self
            .database
            .raw_execute(&format!("DROP TABLE IF EXISTS {MFA_TABLE}"), &[])
            .await;
    }

    async fn rows_for_guard(&self, guard: &str) -> Vec<DbRecord> {
        self.database
            .raw_query(
                r#"
                SELECT actor_id, name, access_token_hash, refresh_token_hash,
                       revoked_at IS NULL AS is_active
                FROM personal_access_tokens
                WHERE guard = $1
                ORDER BY created_at, access_token_hash
                "#,
                &[DbValue::Text(guard.to_string())],
            )
            .await
            .unwrap()
    }

    async fn pat_count(&self) -> i64 {
        self.database
            .raw_query(&format!("SELECT COUNT(*) AS count FROM {PAT_TABLE}"), &[])
            .await
            .unwrap()[0]
            .decode("count")
            .unwrap()
    }

    async fn reset_count(&self, guard_like: &str) -> i64 {
        self.database
            .raw_query(
                &format!("SELECT COUNT(*) AS count FROM {RESET_TABLE} WHERE guard LIKE $1"),
                &[DbValue::Text(guard_like.to_string())],
            )
            .await
            .unwrap()[0]
            .decode("count")
            .unwrap()
    }

    async fn mfa_state(&self, actor: &Actor) -> Option<(bool, i64)> {
        self.database
            .raw_query(
                r#"
                SELECT confirmed_at IS NOT NULL AS confirmed,
                       jsonb_array_length(recovery_codes)::bigint AS recovery_count
                FROM auth_mfa_totp_factors
                WHERE guard = $1 AND actor_id = $2
                "#,
                &[
                    DbValue::Text(actor.guard.to_string()),
                    DbValue::Text(actor.id.clone()),
                ],
            )
            .await
            .unwrap()
            .first()
            .map(|row| {
                (
                    row.decode::<bool>("confirmed").unwrap(),
                    row.decode::<i64>("recovery_count").unwrap(),
                )
            })
    }

    async fn mfa_count_for(&self, actor: &Actor) -> i64 {
        self.database
            .raw_query(
                "SELECT COUNT(*) AS count FROM auth_mfa_totp_factors WHERE guard = $1 AND actor_id = $2",
                &[
                    DbValue::Text(actor.guard.to_string()),
                    DbValue::Text(actor.id.clone()),
                ],
            )
            .await
            .unwrap()[0]
            .decode("count")
            .unwrap()
    }

    async fn cli(&self) -> CliKernel {
        App::builder()
            .load_config_dir(self._dir.path())
            .build_cli_kernel()
            .await
            .unwrap()
    }

    async fn worker(&self) -> WorkerKernel {
        App::builder()
            .load_config_dir(self._dir.path())
            .build_worker_kernel()
            .await
            .unwrap()
    }
}

fn current_totp_step() -> i64 {
    chrono::Utc::now().timestamp() / TOTP_PERIOD_SECONDS
}

fn totp_code(secret: &str, step: i64) -> String {
    let secret = decode_base32(secret);
    let counter = (step as u64).to_be_bytes();
    let mut mac = HmacSha256::new_from_slice(&secret).unwrap();
    mac.update(&counter);
    let result = mac.finalize().into_bytes();
    let offset = (result[result.len() - 1] & 0x0f) as usize;
    let binary = ((u32::from(result[offset]) & 0x7f) << 24)
        | (u32::from(result[offset + 1]) << 16)
        | (u32::from(result[offset + 2]) << 8)
        | u32::from(result[offset + 3]);
    format!("{:06}", binary % 10_u32.pow(TOTP_DIGITS))
}

fn decode_base32(value: &str) -> Vec<u8> {
    let mut buffer = 0u32;
    let mut bits_left = 0u8;
    let mut output = Vec::new();

    for byte in value.bytes().filter(|byte| *byte != b'=') {
        let normalized = byte.to_ascii_uppercase();
        let digit = match normalized {
            b'A'..=b'Z' => normalized - b'A',
            b'2'..=b'7' => normalized - b'2' + 26,
            _ => panic!("invalid base32 test secret"),
        };

        buffer = (buffer << 5) | u32::from(digit);
        bits_left += 5;
        if bits_left >= 8 {
            output.push(((buffer >> (bits_left - 8)) & 0xff) as u8);
            bits_left -= 8;
        }
    }

    output
}

async fn reset_personal_access_tokens(database: &DatabaseManager) {
    database
        .raw_execute(&format!("DROP TABLE IF EXISTS {PAT_TABLE}"), &[])
        .await
        .unwrap();

    // Keep this in sync with the published PAT migration: actor_id is TEXT.
    database
        .raw_execute(
            r#"
            CREATE TABLE personal_access_tokens (
                id UUID PRIMARY KEY DEFAULT uuidv7(),
                guard TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                name TEXT NOT NULL DEFAULT '',
                access_token_hash TEXT NOT NULL,
                refresh_token_hash TEXT,
                abilities JSONB NOT NULL DEFAULT '[]',
                expires_at TIMESTAMPTZ NOT NULL,
                refresh_expires_at TIMESTAMPTZ,
                last_used_at TIMESTAMPTZ,
                revoked_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
            &[],
        )
        .await
        .unwrap();

    database
        .raw_execute(
            "CREATE INDEX idx_pat_access_hash ON personal_access_tokens (access_token_hash) WHERE revoked_at IS NULL",
            &[],
        )
        .await
        .unwrap();
    database
        .raw_execute(
            "CREATE INDEX idx_pat_refresh_hash ON personal_access_tokens (refresh_token_hash) WHERE revoked_at IS NULL",
            &[],
        )
        .await
        .unwrap();
    database
        .raw_execute(
            "CREATE INDEX idx_pat_actor ON personal_access_tokens (guard, actor_id)",
            &[],
        )
        .await
        .unwrap();
}

async fn reset_password_reset_tokens(database: &DatabaseManager) {
    database
        .raw_execute(&format!("DROP TABLE IF EXISTS {RESET_TABLE}"), &[])
        .await
        .unwrap();

    database
        .raw_execute(
            r#"
            CREATE TABLE password_reset_tokens (
                email TEXT NOT NULL,
                guard TEXT NOT NULL,
                token_hash TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
            &[],
        )
        .await
        .unwrap();

    database
        .raw_execute(
            "CREATE UNIQUE INDEX idx_password_reset_email_guard ON password_reset_tokens (email, guard)",
            &[],
        )
        .await
        .unwrap();
}

async fn reset_mfa_totp_factors(database: &DatabaseManager) {
    database
        .raw_execute(&format!("DROP TABLE IF EXISTS {MFA_TABLE}"), &[])
        .await
        .unwrap();

    database
        .raw_execute(
            r#"
            CREATE TABLE auth_mfa_totp_factors (
                id UUID PRIMARY KEY DEFAULT uuidv7(),
                guard TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                secret_ciphertext TEXT NOT NULL,
                confirmed_at TIMESTAMPTZ,
                recovery_codes JSONB NOT NULL DEFAULT '[]',
                last_used_step BIGINT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
            &[],
        )
        .await
        .unwrap();

    database
        .raw_execute(
            "CREATE UNIQUE INDEX idx_auth_mfa_totp_guard_actor ON auth_mfa_totp_factors (guard, actor_id)",
            &[],
        )
        .await
        .unwrap();
}

#[derive(Clone, Debug)]
struct DirectManagerActor;

impl Model for DirectManagerActor {
    type Lifecycle = NoModelLifecycle;

    fn table_meta() -> &'static TableMeta<Self> {
        unimplemented!("token acceptance tests do not query actor model tables")
    }
}

#[async_trait]
impl Authenticatable for DirectManagerActor {
    fn guard() -> GuardId {
        GuardId::new("text_api")
    }
}

#[tokio::test]
async fn password_reset_token_validation_is_atomic_single_use() {
    let _guard = token_lock().await;
    let Some(runtime) = TokenTestRuntime::new().await else {
        return;
    };

    let resets = runtime.app.password_resets().unwrap();
    let token = resets
        .create_token::<DirectManagerActor>("race@example.com")
        .await
        .unwrap();

    let first = resets.clone();
    let second = resets.clone();
    let (left, right) = tokio::join!(
        first.validate_token::<DirectManagerActor>("race@example.com", &token),
        second.validate_token::<DirectManagerActor>("race@example.com", &token),
    );

    let successes = [left.is_ok(), right.is_ok()]
        .into_iter()
        .filter(|ok| *ok)
        .count();
    assert_eq!(successes, 1);
    assert_eq!(runtime.reset_count("text_api").await, 0);

    runtime.cleanup().await;
}

#[tokio::test]
async fn email_verification_token_validation_is_atomic_single_use() {
    let _guard = token_lock().await;
    let Some(runtime) = TokenTestRuntime::new().await else {
        return;
    };

    let verification = runtime.app.email_verification().unwrap();
    let token = verification
        .create_token::<DirectManagerActor>("verify@example.com")
        .await
        .unwrap();

    verification
        .validate_token::<DirectManagerActor>("verify@example.com", &token)
        .await
        .unwrap();
    let error = verification
        .validate_token::<DirectManagerActor>("verify@example.com", &token)
        .await
        .unwrap_err();

    assert!(error
        .to_string()
        .contains("invalid or expired verification token"));
    assert_eq!(runtime.reset_count("verify:%").await, 0);

    runtime.cleanup().await;
}

#[tokio::test]
async fn mfa_totp_factor_lifecycle_persists_and_consumes_recovery_codes() {
    let _guard = token_lock().await;
    let Some(runtime) = TokenTestRuntime::new_with_config(
        r#"
        [crypt]
        key = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="

        [auth.mfa]
        recovery_codes = 2
        "#,
    )
    .await
    else {
        return;
    };

    let actor = Actor::new("mfa-actor-1", DirectManagerActor::guard());
    let totp = MfaManager::new(&runtime.app).unwrap().totp();
    let challenge = totp.enroll(&actor).await.unwrap();
    assert_eq!(runtime.mfa_state(&actor).await, Some((false, 0)));

    let confirm_code = totp_code(&challenge.secret, current_totp_step());
    totp.confirm(&actor, &confirm_code).await.unwrap();

    let recovery_codes = totp
        .regenerate_recovery_codes(
            &actor,
            &totp_code(&challenge.secret, current_totp_step() + 1),
        )
        .await
        .unwrap();
    assert_eq!(recovery_codes.len(), 2);

    totp.verify(&actor, &recovery_codes[0]).await.unwrap();
    assert_eq!(runtime.mfa_state(&actor).await, Some((true, 1)));

    let replayed = totp.verify(&actor, &recovery_codes[0]).await.unwrap_err();
    assert!(replayed.to_string().contains("Invalid multi-factor"));

    totp.disable(&actor, &recovery_codes[1]).await.unwrap();
    assert_eq!(runtime.mfa_count_for(&actor).await, 0);

    runtime.cleanup().await;
}

#[derive(Clone, Debug)]
struct ExternalActorUser {
    id: i64,
    external_actor_id: String,
}

impl Model for ExternalActorUser {
    type Lifecycle = NoModelLifecycle;

    fn table_meta() -> &'static TableMeta<Self> {
        unimplemented!("token acceptance tests do not query actor model tables")
    }
}

#[async_trait]
impl Authenticatable for ExternalActorUser {
    fn guard() -> GuardId {
        GuardId::new("external_api")
    }
}

impl HasToken for ExternalActorUser {
    fn token_actor_id(&self) -> String {
        self.external_actor_id.clone()
    }
}

#[derive(Clone, Debug)]
struct UuidBackedActorUser {
    id: ModelId<UuidBackedActorUser>,
    _email: String,
}

impl Model for UuidBackedActorUser {
    type Lifecycle = NoModelLifecycle;

    fn table_meta() -> &'static TableMeta<Self> {
        static COLUMNS: [ColumnInfo; 2] = [
            ColumnInfo::new("id", DbType::Uuid),
            ColumnInfo::new("email", DbType::Text),
        ];
        static TABLE: OnceLock<TableMeta<UuidBackedActorUser>> = OnceLock::new();
        TABLE.get_or_init(|| {
            TableMeta::new(
                "token_uuid_actor_users",
                &COLUMNS,
                "id",
                ModelPrimaryKeyStrategy::Manual,
                ModelBehavior::new(ModelFeatureSetting::Default, ModelFeatureSetting::Default),
                |record| {
                    Ok(UuidBackedActorUser {
                        id: record.decode("id")?,
                        _email: record.decode("email")?,
                    })
                },
            )
        })
    }
}

#[async_trait]
impl Authenticatable for UuidBackedActorUser {
    fn guard() -> GuardId {
        GuardId::new("uuid_api")
    }
}

impl HasToken for UuidBackedActorUser {
    fn token_actor_id(&self) -> String {
        self.id.to_string()
    }
}

#[tokio::test]
async fn token_manager_issue_refresh_and_revoke_all_use_text_actor_ids() {
    let _guard = token_lock().await;
    let Some(runtime) = TokenTestRuntime::new().await else {
        return;
    };

    let tokens = runtime.app.tokens().unwrap();
    let pair = tokens
        .issue_named::<DirectManagerActor>("acct-42", "cli")
        .await
        .unwrap();

    let actor = tokens.validate(&pair.access_token).await.unwrap().unwrap();
    assert_eq!(actor.id, "acct-42");
    assert_eq!(actor.guard, DirectManagerActor::guard());

    let initial_rows = runtime.rows_for_guard("text_api").await;
    assert_eq!(initial_rows.len(), 1);
    assert_eq!(
        initial_rows[0].decode::<String>("actor_id").unwrap(),
        "acct-42"
    );
    assert_eq!(initial_rows[0].decode::<String>("name").unwrap(), "cli");
    assert!(initial_rows[0].decode::<bool>("is_active").unwrap());

    let refreshed = tokens.refresh(&pair.refresh_token).await.unwrap();

    assert!(tokens.validate(&pair.access_token).await.unwrap().is_none());
    let refreshed_actor = tokens
        .validate(&refreshed.access_token)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(refreshed_actor.id, "acct-42");

    let refreshed_rows = runtime.rows_for_guard("text_api").await;
    assert_eq!(refreshed_rows.len(), 2);
    assert_eq!(
        refreshed_rows
            .iter()
            .filter(|row| row.decode::<bool>("is_active").unwrap())
            .count(),
        1
    );

    let revoked = tokens
        .revoke_all::<DirectManagerActor>("acct-42")
        .await
        .unwrap();
    assert_eq!(revoked, 1);
    assert!(tokens
        .validate(&refreshed.access_token)
        .await
        .unwrap()
        .is_none());

    runtime.cleanup().await;
}

#[tokio::test]
async fn has_token_uses_custom_token_actor_id_for_storage_and_revocation() {
    let _guard = token_lock().await;
    let Some(runtime) = TokenTestRuntime::new().await else {
        return;
    };

    let user = ExternalActorUser {
        id: 7,
        external_actor_id: "merchant:store-9".to_string(),
    };

    let pair = user
        .create_token_named(&runtime.app, "dashboard")
        .await
        .unwrap();
    let actor = runtime
        .app
        .tokens()
        .unwrap()
        .validate(&pair.access_token)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actor.id, user.external_actor_id);

    let rows = runtime.rows_for_guard("external_api").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].decode::<String>("actor_id").unwrap(),
        "merchant:store-9"
    );
    assert_ne!(
        rows[0].decode::<String>("actor_id").unwrap(),
        user.id.to_string()
    );

    let revoked = user.revoke_all_tokens(&runtime.app).await.unwrap();
    assert_eq!(revoked, 1);
    assert!(runtime
        .app
        .tokens()
        .unwrap()
        .validate(&pair.access_token)
        .await
        .unwrap()
        .is_none());

    runtime.cleanup().await;
}

#[tokio::test]
async fn uuid_backed_authenticatables_store_actor_ids_as_text() {
    let _guard = token_lock().await;
    let Some(runtime) = TokenTestRuntime::new().await else {
        return;
    };

    let user = UuidBackedActorUser {
        id: ModelId::generate(),
        _email: "uuid@example.com".to_string(),
    };
    let actor_id = user.id.to_string();

    let pair = user.create_token(&runtime.app).await.unwrap();
    let actor = runtime
        .app
        .tokens()
        .unwrap()
        .validate(&pair.access_token)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actor.id, actor_id);

    let rows = runtime.rows_for_guard("uuid_api").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].decode::<String>("actor_id").unwrap(), actor_id);
    assert!(rows[0].decode::<bool>("is_active").unwrap());

    let revoked = user.revoke_all_tokens(&runtime.app).await.unwrap();
    assert_eq!(revoked, 1);
    assert!(runtime
        .app
        .tokens()
        .unwrap()
        .validate(&pair.access_token)
        .await
        .unwrap()
        .is_none());

    runtime.cleanup().await;
}

#[tokio::test]
async fn token_ttl_can_be_overridden_per_guard() {
    let _guard = token_lock().await;
    let Some(runtime) = TokenTestRuntime::new_with_config(
        r#"
        [auth.tokens]
        access_token_ttl_minutes = 15
        refresh_token_ttl_days = 30

        [auth.tokens.guards.text_api]
        access_token_ttl_minutes = 43200
        refresh_token_ttl_days = 3
        "#,
    )
    .await
    else {
        return;
    };

    let tokens = runtime.app.tokens().unwrap();
    let pair = tokens
        .issue_named::<DirectManagerActor>("acct-ttl", "ttl")
        .await
        .unwrap();

    assert_eq!(pair.expires_in, 43_200 * 60);

    runtime.cleanup().await;
}

#[tokio::test]
async fn token_prune_cli_uses_safe_prune_path() {
    let _guard = token_lock().await;
    let Some(runtime) = TokenTestRuntime::new().await else {
        return;
    };

    let tokens = runtime.app.tokens().unwrap();
    let old = tokens
        .issue_named::<DirectManagerActor>("old", "old")
        .await
        .unwrap();
    let active = tokens
        .issue_named::<DirectManagerActor>("active", "active")
        .await
        .unwrap();
    mark_access_token_expired(&runtime.database, &old.access_token, 31).await;
    assert!(tokens
        .validate(&active.access_token)
        .await
        .unwrap()
        .is_some());
    assert_eq!(runtime.pat_count().await, 2);

    runtime
        .cli()
        .await
        .run_with_args(["foundry", "token:prune", "--days", "30"])
        .await
        .unwrap();

    assert_eq!(runtime.pat_count().await, 1);
    assert!(tokens
        .validate(&active.access_token)
        .await
        .unwrap()
        .is_some());

    runtime.cleanup().await;
}

#[tokio::test]
async fn worker_prunes_auth_credentials_with_retention_and_batches() {
    let _guard = token_lock().await;
    let Some(runtime) = TokenTestRuntime::new_with_config(
        r#"
        [jobs]
        history_retention_days = 0

        [auth.tokens]
        prune_retention_days = 30
        prune_interval_ms = 1
        prune_batch_size = 10

        [auth.password_resets]
        expiry_minutes = 60
        prune_interval_ms = 1
        prune_batch_size = 10

        [auth.email_verification]
        expiry_minutes = 60
        prune_interval_ms = 1
        prune_batch_size = 10
        "#,
    )
    .await
    else {
        return;
    };

    let tokens = runtime.app.tokens().unwrap();
    let old = tokens
        .issue_named::<DirectManagerActor>("old-worker", "old")
        .await
        .unwrap();
    let active = tokens
        .issue_named::<DirectManagerActor>("active-worker", "active")
        .await
        .unwrap();
    mark_access_token_expired(&runtime.database, &old.access_token, 31).await;
    insert_reset_token(&runtime.database, "reset-old@example.com", "text_api", 120).await;
    insert_reset_token(&runtime.database, "reset-new@example.com", "text_api", 10).await;
    insert_reset_token(
        &runtime.database,
        "verify-old@example.com",
        "verify:text_api",
        120,
    )
    .await;
    insert_reset_token(
        &runtime.database,
        "verify-new@example.com",
        "verify:text_api",
        10,
    )
    .await;

    runtime.worker().await.run_once().await.unwrap();

    assert_eq!(runtime.pat_count().await, 1);
    assert!(tokens
        .validate(&active.access_token)
        .await
        .unwrap()
        .is_some());
    assert_eq!(runtime.reset_count("text_api").await, 1);
    assert_eq!(runtime.reset_count("verify:%").await, 1);

    runtime.cleanup().await;
}

#[tokio::test]
async fn worker_auth_pruning_respects_zero_disable_and_distributed_lock() {
    let _guard = token_lock().await;
    let Some(runtime) = TokenTestRuntime::new_with_config(
        r#"
        [jobs]
        history_retention_days = 0

        [auth.tokens]
        prune_retention_days = 30
        prune_interval_ms = 1
        prune_batch_size = 10

        [auth.password_resets]
        expiry_minutes = 0
        prune_interval_ms = 1
        prune_batch_size = 10

        [auth.email_verification]
        expiry_minutes = 0
        prune_interval_ms = 1
        prune_batch_size = 10
        "#,
    )
    .await
    else {
        return;
    };

    let worker = runtime.worker().await;
    let app = worker.app().clone();
    let lock = app
        .lock()
        .unwrap()
        .acquire("auth:tokens_prune", std::time::Duration::from_secs(60))
        .await
        .unwrap()
        .expect("test should acquire prune lock");

    let tokens = runtime.app.tokens().unwrap();
    let old = tokens
        .issue_named::<DirectManagerActor>("old-locked", "old")
        .await
        .unwrap();
    mark_access_token_expired(&runtime.database, &old.access_token, 31).await;
    insert_reset_token(
        &runtime.database,
        "reset-disabled@example.com",
        "text_api",
        120,
    )
    .await;
    insert_reset_token(
        &runtime.database,
        "verify-disabled@example.com",
        "verify:text_api",
        120,
    )
    .await;

    worker.run_once().await.unwrap();

    assert_eq!(runtime.pat_count().await, 1);
    assert_eq!(runtime.reset_count("text_api").await, 1);
    assert_eq!(runtime.reset_count("verify:%").await, 1);

    drop(lock);
    runtime.cleanup().await;
}

async fn mark_access_token_expired(database: &DatabaseManager, access_token: &str, days_ago: i64) {
    database
        .raw_execute(
            r#"
            UPDATE personal_access_tokens
            SET expires_at = NOW() - ($2::double precision * INTERVAL '1 day'),
                created_at = NOW() - ($2::double precision * INTERVAL '1 day')
            WHERE access_token_hash = $1
            "#,
            &[
                DbValue::Text(foundry::sha256_hex_str(access_token)),
                DbValue::Int64(days_ago),
            ],
        )
        .await
        .unwrap();
}

async fn insert_reset_token(
    database: &DatabaseManager,
    email: &str,
    guard: &str,
    minutes_ago: i64,
) {
    database
        .raw_execute(
            r#"
            INSERT INTO password_reset_tokens (email, guard, token_hash, created_at)
            VALUES ($1, $2, $3, NOW() - ($4::double precision * INTERVAL '1 minute'))
            "#,
            &[
                DbValue::Text(email.to_string()),
                DbValue::Text(guard.to_string()),
                DbValue::Text(format!("hash:{email}")),
                DbValue::Int64(minutes_ago),
            ],
        )
        .await
        .unwrap();
}
