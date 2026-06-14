use async_trait::async_trait;
use foundry::prelude::*;

pub struct Entry;

#[async_trait]
impl MigrationFile for Entry {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            r#"
            CREATE TABLE attachments (
                id UUID PRIMARY KEY DEFAULT uuidv7(),
                attachable_type TEXT NOT NULL,
                attachable_id UUID NOT NULL,
                collection TEXT NOT NULL DEFAULT 'default',
                disk TEXT NOT NULL,
                path TEXT NOT NULL,
                name TEXT NOT NULL,
                original_name TEXT,
                mime_type TEXT,
                size BIGINT NOT NULL DEFAULT 0,
                sort_order INT NOT NULL DEFAULT 0,
                custom_properties JSONB NOT NULL DEFAULT '{}',
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ
            )
            "#,
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_attachments_poly ON attachments (attachable_type, attachable_id, collection)",
            &[],
        )
        .await?;

        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute("DROP TABLE IF EXISTS attachments", &[])
            .await?;
        Ok(())
    }
}
