use async_trait::async_trait;
use foundry::prelude::*;

pub struct Entry;

#[async_trait]
impl MigrationFile for Entry {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            r#"
            CREATE TABLE metadata (
                id UUID PRIMARY KEY DEFAULT uuidv7(),
                metadatable_type TEXT NOT NULL,
                metadatable_id UUID NOT NULL,
                key TEXT NOT NULL,
                value JSONB,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ
            )
            "#,
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE UNIQUE INDEX idx_metadata_unique ON metadata (metadatable_type, metadatable_id, key)",
            &[],
        )
        .await?;

        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute("DROP TABLE IF EXISTS metadata", &[])
            .await?;
        Ok(())
    }
}
