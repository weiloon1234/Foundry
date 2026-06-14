use async_trait::async_trait;
use foundry::prelude::*;

pub struct Entry;

#[async_trait]
impl MigrationFile for Entry {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            r#"
            CREATE TABLE audit_logs (
                id UUID PRIMARY KEY DEFAULT uuidv7(),
                event_type TEXT NOT NULL,
                subject_model TEXT NOT NULL,
                subject_table TEXT NOT NULL,
                subject_id TEXT NOT NULL,
                area TEXT,
                actor_guard TEXT,
                actor_id TEXT,
                request_id TEXT,
                ip TEXT,
                user_agent TEXT,
                before_data JSONB,
                after_data JSONB,
                changes JSONB,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_audit_logs_subject ON audit_logs (subject_table, subject_id)",
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_audit_logs_actor ON audit_logs (actor_guard, actor_id)",
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_audit_logs_area_created_at ON audit_logs (area, created_at)",
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_audit_logs_event_type ON audit_logs (event_type)",
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_audit_logs_created_at ON audit_logs (created_at)",
            &[],
        )
        .await?;

        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute("DROP TABLE IF EXISTS audit_logs", &[])
            .await?;
        Ok(())
    }
}
