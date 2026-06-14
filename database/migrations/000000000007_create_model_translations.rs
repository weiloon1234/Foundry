use async_trait::async_trait;
use foundry::prelude::*;

pub struct Entry;

#[async_trait]
impl MigrationFile for Entry {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            r#"
            CREATE TABLE model_translations (
                id UUID PRIMARY KEY DEFAULT uuidv7(),
                translatable_type TEXT NOT NULL,
                translatable_id UUID NOT NULL,
                locale TEXT NOT NULL,
                field TEXT NOT NULL,
                value TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ
            )
            "#,
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE UNIQUE INDEX idx_translations_unique ON model_translations (translatable_type, translatable_id, locale, field)",
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_translations_lookup ON model_translations (translatable_type, translatable_id, locale)",
            &[],
        )
        .await?;

        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute("DROP TABLE IF EXISTS model_translations", &[])
            .await?;
        Ok(())
    }
}
