use async_trait::async_trait;
use foundry::prelude::*;

pub struct Entry;

#[async_trait]
impl MigrationFile for Entry {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            r#"
            CREATE TABLE settings (
                id UUID PRIMARY KEY DEFAULT uuidv7(),
                key TEXT NOT NULL,
                value JSONB,
                setting_type TEXT NOT NULL DEFAULT 'text',
                parameters JSONB NOT NULL DEFAULT '{}',
                group_name TEXT NOT NULL DEFAULT 'general',
                label TEXT NOT NULL DEFAULT '',
                description TEXT,
                sort_order INT NOT NULL DEFAULT 0,
                is_public BOOLEAN NOT NULL DEFAULT false,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ
            )
            "#,
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE UNIQUE INDEX idx_settings_key ON settings (key)",
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_settings_group ON settings (group_name, sort_order)",
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_settings_public ON settings (is_public) WHERE is_public = true",
            &[],
        )
        .await?;

        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute("DROP TABLE IF EXISTS settings", &[])
            .await?;
        Ok(())
    }
}
