use async_trait::async_trait;
use foundry::prelude::*;

pub struct Entry;

#[async_trait]
impl MigrationFile for Entry {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
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
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_pat_access_hash ON personal_access_tokens (access_token_hash) WHERE revoked_at IS NULL",
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_pat_refresh_hash ON personal_access_tokens (refresh_token_hash) WHERE revoked_at IS NULL",
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_pat_actor ON personal_access_tokens (guard, actor_id)",
            &[],
        )
        .await?;

        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute("DROP TABLE IF EXISTS personal_access_tokens", &[])
            .await?;
        Ok(())
    }
}
