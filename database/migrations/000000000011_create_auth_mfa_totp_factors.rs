use async_trait::async_trait;
use foundry::prelude::*;

pub struct Entry;

#[async_trait]
impl MigrationFile for Entry {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            r#"
            CREATE TABLE IF NOT EXISTS auth_mfa_totp_factors (
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
        .await?;

        ctx.raw_execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_auth_mfa_totp_guard_actor ON auth_mfa_totp_factors (guard, actor_id)",
            &[],
        )
        .await?;

        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute("DROP TABLE IF EXISTS auth_mfa_totp_factors", &[])
            .await?;
        Ok(())
    }
}
