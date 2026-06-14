use async_trait::async_trait;
use foundry::prelude::*;

pub struct Entry;

#[async_trait]
impl MigrationFile for Entry {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            "CREATE INDEX IF NOT EXISTS idx_job_history_created_at ON job_history (created_at)",
            &[],
        )
        .await?;
        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute("DROP INDEX IF EXISTS idx_job_history_created_at", &[])
            .await?;
        Ok(())
    }
}
