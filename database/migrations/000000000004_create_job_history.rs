use async_trait::async_trait;
use foundry::prelude::*;

pub struct Entry;

#[async_trait]
impl MigrationFile for Entry {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            r#"
            CREATE TABLE job_history (
                id UUID PRIMARY KEY DEFAULT uuidv7(),
                job_id TEXT NOT NULL,
                queue TEXT NOT NULL,
                status TEXT NOT NULL,
                payload JSONB,
                attempt INT NOT NULL DEFAULT 1,
                error TEXT,
                started_at TIMESTAMPTZ,
                completed_at TIMESTAMPTZ,
                duration_ms BIGINT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_job_history_job_id ON job_history (job_id)",
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_job_history_status ON job_history (status)",
            &[],
        )
        .await?;

        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute("DROP TABLE IF EXISTS job_history", &[])
            .await?;
        Ok(())
    }
}
