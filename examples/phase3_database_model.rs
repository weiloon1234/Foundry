use async_trait::async_trait;
use foundry::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UserStatus {
    Active,
    Disabled,
}

impl ToDbValue for UserStatus {
    fn to_db_value(self) -> DbValue {
        match self {
            Self::Active => "active".into(),
            Self::Disabled => "disabled".into(),
        }
    }
}

impl FromDbValue for UserStatus {
    fn from_db_value(value: &DbValue) -> Result<Self> {
        match value {
            DbValue::Text(value) if value == "active" => Ok(Self::Active),
            DbValue::Text(value) if value == "disabled" => Ok(Self::Disabled),
            _ => Err(Error::message("unknown user status")),
        }
    }
}

#[derive(foundry::Model)]
#[foundry(table = "users", lifecycle = UserLifecycle, soft_deletes = true)]
struct User {
    id: ModelId<User>,
    email: String,
    #[foundry(write_mutator = "hash_password")]
    #[foundry(read_accessor = "masked_password")]
    password: String,
    #[foundry(db_type = "text")]
    status: UserStatus,
    login_count: i64,
    nickname: Option<String>,
    created_at: DateTime,
    updated_at: DateTime,
    deleted_at: Option<DateTime>,
}

struct UserLifecycle;

#[async_trait]
impl ModelLifecycle<User> for UserLifecycle {
    async fn creating(
        _context: &ModelHookContext<'_>,
        draft: &mut CreateDraft<User>,
    ) -> Result<()> {
        if draft.pending_record().get("nickname").is_none() {
            draft.set(User::NICKNAME, "new-user");
        }
        Ok(())
    }

    async fn updating(
        _context: &ModelHookContext<'_>,
        _current: &User,
        draft: &mut UpdateDraft<User>,
    ) -> Result<()> {
        if draft.pending_record().get("nickname").is_none() {
            draft.set(User::NICKNAME, "freshly-updated");
        }
        Ok(())
    }
}

impl User {
    async fn hash_password(ctx: &ModelHookContext<'_>, value: String) -> Result<String> {
        ctx.app().hash()?.hash(&value)
    }

    fn masked_password(&self) -> String {
        "********".to_string()
    }

    pub fn display_name(&self) -> &str {
        &self.email
    }
}

async fn aggregate_examples(db: &DatabaseManager) -> Result<()> {
    let active_count = User::query()
        .where_(User::STATUS.eq(UserStatus::Active))
        .count(db)
        .await?;
    let login_sum = User::query().sum(db, User::LOGIN_COUNT).await?;

    println!("active users = {active_count}");
    println!("sum of login counts = {:?}", login_sum);

    Ok(())
}

async fn write_examples(app: &AppContext) -> Result<()> {
    let created = User::create()
        .set(User::EMAIL, "foundry@example.com")
        .set(User::PASSWORD, "secret-password")
        .set(User::STATUS, UserStatus::Active)
        .set(User::LOGIN_COUNT, 1_i64)
        .save(app)
        .await?;

    let updated = created
        .update()
        .set(User::STATUS, UserStatus::Disabled)
        .save(app)
        .await?;

    updated.delete().execute(app).await?;

    User::restore()
        .where_(User::ID.eq(created.id))
        .execute(app)
        .await?;

    User::force_delete()
        .where_(User::ID.eq(created.id))
        .execute(app)
        .await?;

    Ok(())
}

fn main() -> Result<()> {
    let insert_id = ModelId::<User>::generate();
    let bulk_id = ModelId::<User>::generate();
    let upsert_id = ModelId::<User>::generate();

    let list_users = User::query()
        .where_(User::STATUS.eq(UserStatus::Active))
        .order_by(User::ID.asc());
    let _visible_users = User::query();
    let _all_users = User::query().with_trashed();
    let _trashed_users = User::query().only_trashed();
    let _insert_user = User::create()
        .set(User::ID, insert_id)
        .set(User::EMAIL, "foundry@example.com")
        .set(User::PASSWORD, "secret-password")
        .set(User::STATUS, UserStatus::Active)
        .set(User::LOGIN_COUNT, 10_i64)
        .set(User::NICKNAME, "captain");
    let _bulk_insert = User::create_many()
        .row(|row| {
            row.set(User::ID, bulk_id)
                .set(User::EMAIL, "ops@example.com")
                .set(User::PASSWORD, "ops-password")
                .set(User::STATUS, UserStatus::Disabled)
                .set(User::LOGIN_COUNT, 5_i64)
                .set(User::NICKNAME, None::<String>)
        })
        .row(|row| {
            row.set(User::EMAIL, "dev@example.com")
                .set(User::PASSWORD, "dev-password")
                .set(User::STATUS, UserStatus::Active)
                .set(User::LOGIN_COUNT, 7_i64)
                .set(User::NICKNAME, "ally")
        });
    let _upsert_user = User::create()
        .set(User::ID, upsert_id)
        .set(User::EMAIL, "dev-updated@example.com")
        .set(User::PASSWORD, "dev-updated-password")
        .set(User::STATUS, UserStatus::Active)
        .set(User::LOGIN_COUNT, 11_i64)
        .set(User::NICKNAME, "vip")
        .on_conflict_columns([User::ID])
        .do_update()
        .set_excluded(User::EMAIL)
        .set_excluded(User::STATUS)
        .set_excluded(User::LOGIN_COUNT)
        .set_excluded(User::NICKNAME);
    let _patch_user = User::update()
        .set(User::STATUS, UserStatus::Disabled)
        .set(User::LOGIN_COUNT, 12_i64)
        .set_null(User::NICKNAME)
        .where_(User::ID.eq(upsert_id));
    let _restore_user = User::restore().where_(User::ID.eq(upsert_id));
    let _force_delete_user = User::force_delete().where_(User::ID.eq(upsert_id));

    let _generated_id_insert = User::create()
        .set(User::EMAIL, "auto-id@example.com")
        .set(User::PASSWORD, "auto-secret")
        .set(User::STATUS, UserStatus::Active)
        .set(User::LOGIN_COUNT, 3_i64);

    println!("{:?}", list_users.ast());
    let _masked = User {
        id: insert_id,
        email: "foundry@example.com".to_string(),
        password: "$argon2id$example".to_string(),
        status: UserStatus::Active,
        login_count: 1,
        nickname: None,
        created_at: DateTime::parse("2026-01-01T00:00:00Z").unwrap(),
        updated_at: DateTime::parse("2026-01-01T00:00:00Z").unwrap(),
        deleted_at: None,
    };
    let _ = _masked.password_accessed();
    let _ = _masked.display_name();
    let _ = aggregate_examples;
    let _ = write_examples;

    Ok(())
}
