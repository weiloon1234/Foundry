use async_trait::async_trait;
use foundry::prelude::*;

pub struct Entry;

#[async_trait]
impl MigrationFile for Entry {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            r#"
            CREATE TABLE notifications (
                id UUID PRIMARY KEY DEFAULT uuidv7(),
                notifiable_id TEXT NOT NULL,
                type TEXT NOT NULL,
                data JSONB NOT NULL DEFAULT '{}',
                read_at TIMESTAMPTZ,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_notifications_notifiable ON notifications (notifiable_id, read_at)",
            &[],
        )
        .await?;

        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute("DROP TABLE IF EXISTS notifications", &[])
            .await?;
        Ok(())
    }
}
