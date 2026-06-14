use async_trait::async_trait;
use foundry::prelude::*;

#[derive(foundry::Model)]
#[foundry(table = "users", lifecycle = UserLifecycle)]
struct User {
    id: ModelId<User>,
    #[foundry(column = "user_email")]
    #[foundry(write_mutator = "normalize_email")]
    #[foundry(read_accessor = "normalized_email")]
    email: String,
    active: bool,
    metadata: serde_json::Value,
    created_at: DateTime,
    #[foundry(write_mutator = "normalize_nickname")]
    nickname: Option<String>,
    merchants: Loaded<Vec<Merchant>>,
}

#[derive(foundry::Model)]
#[foundry(table = "merchants")]
struct Merchant {
    id: ModelId<Merchant>,
}

#[derive(foundry::Model)]
#[foundry(table = "external_accounts", primary_key = "public_id")]
struct ExternalAccount {
    public_id: ModelId<ExternalAccount>,
    email: String,
}

#[derive(foundry::Model)]
#[foundry(table = "legacy_users", primary_key_strategy = "manual")]
struct LegacyUser {
    id: i64,
    email: String,
}

#[derive(foundry::Model)]
#[foundry(table = "api_tokens", primary_key_strategy = "manual", audit = false)]
struct ApiToken {
    id: String,
    #[foundry(audit_exclude)]
    secret: String,
}

struct UserLifecycle;

#[async_trait]
impl ModelLifecycle<User> for UserLifecycle {}

impl User {
    async fn normalize_email(_ctx: &ModelHookContext<'_>, value: String) -> Result<String> {
        Ok(value.trim().to_lowercase())
    }

    fn normalized_email(&self) -> String {
        self.email.trim().to_lowercase()
    }

    async fn normalize_nickname(
        _ctx: &ModelHookContext<'_>,
        value: Option<String>,
    ) -> Result<Option<String>> {
        Ok(value.map(|nickname| nickname.trim().to_lowercase()))
    }
}

fn main() {
    let _ = User::ID;
    let _ = User::EMAIL;
    let _ = User::table_meta();
    let _ = User::create();
    let user = User {
        id: ModelId::generate(),
        email: " USER@EXAMPLE.COM ".to_string(),
        active: true,
        metadata: serde_json::json!({}),
        created_at: DateTime::parse("2026-01-01T00:00:00Z").unwrap(),
        nickname: None,
        merchants: Loaded::Unloaded,
    };
    let _ = user.email_accessed();
    let _ = ExternalAccount::PUBLIC_ID;
    let _ = LegacyUser::ID;
    let _ = ApiToken::ID;
    let _ = ApiToken::audit_enabled();
    let _ = ApiToken::audit_excluded_fields();
}
