use async_trait::async_trait;
use foundry::prelude::*;

pub struct Entry;

#[async_trait]
impl MigrationFile for Entry {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            "ALTER TABLE notifications ADD COLUMN IF NOT EXISTS notifiable_type TEXT NOT NULL DEFAULT 'default'",
            &[],
        )
        .await?;
        ctx.raw_execute("DROP INDEX IF EXISTS idx_notifications_notifiable", &[])
            .await?;
        ctx.raw_execute(
            "CREATE INDEX idx_notifications_notifiable ON notifications (notifiable_type, notifiable_id, created_at DESC, id DESC)",
            &[],
        )
        .await?;
        ctx.raw_execute(
            "CREATE INDEX IF NOT EXISTS idx_notifications_unread ON notifications (notifiable_type, notifiable_id, created_at DESC, id DESC) WHERE read_at IS NULL",
            &[],
        )
        .await?;
        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute("DROP INDEX IF EXISTS idx_notifications_unread", &[])
            .await?;
        ctx.raw_execute("DROP INDEX IF EXISTS idx_notifications_notifiable", &[])
            .await?;
        ctx.raw_execute(
            "CREATE INDEX idx_notifications_notifiable ON notifications (notifiable_id, read_at)",
            &[],
        )
        .await?;
        ctx.raw_execute(
            "ALTER TABLE notifications DROP COLUMN IF EXISTS notifiable_type",
            &[],
        )
        .await?;
        Ok(())
    }
}
