use async_trait::async_trait;
use foundry::prelude::*;

pub struct Entry;

#[async_trait]
impl MigrationFile for Entry {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            r#"
            CREATE TABLE IF NOT EXISTS password_reset_tokens (
                email TEXT NOT NULL,
                guard TEXT NOT NULL,
                token_hash TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
            &[],
        )
        .await?;
        ctx.raw_execute(
            r#"
            CREATE UNIQUE INDEX IF NOT EXISTS idx_password_reset_email_guard
                ON password_reset_tokens (email, guard)
            "#,
            &[],
        )
        .await?;
        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute("DROP TABLE IF EXISTS password_reset_tokens", &[])
            .await?;
        Ok(())
    }
}
