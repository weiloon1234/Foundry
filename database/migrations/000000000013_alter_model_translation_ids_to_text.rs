use async_trait::async_trait;
use foundry::prelude::*;

pub struct Entry;

#[async_trait]
impl MigrationFile for Entry {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            r#"
            DO $migration$
            BEGIN
                IF EXISTS (
                    SELECT 1
                    FROM information_schema.columns
                    WHERE table_schema = current_schema()
                      AND table_name = 'model_translations'
                      AND column_name = 'translatable_id'
                      AND data_type = 'uuid'
                ) THEN
                    ALTER TABLE model_translations
                        ALTER COLUMN translatable_id TYPE TEXT
                        USING translatable_id::text;
                END IF;
            END
            $migration$
            "#,
            &[],
        )
        .await?;

        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            r#"
            DO $migration$
            BEGIN
                IF EXISTS (
                    SELECT 1
                    FROM information_schema.columns
                    WHERE table_schema = current_schema()
                      AND table_name = 'model_translations'
                      AND column_name = 'translatable_id'
                      AND data_type = 'text'
                ) THEN
                    IF EXISTS (
                        SELECT 1
                        FROM model_translations
                        WHERE translatable_id !~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                    ) THEN
                        RAISE EXCEPTION 'cannot restore UUID model_translations.translatable_id while non-UUID IDs exist';
                    END IF;

                    ALTER TABLE model_translations
                        ALTER COLUMN translatable_id TYPE UUID
                        USING translatable_id::uuid;
                END IF;
            END
            $migration$
            "#,
            &[],
        )
        .await?;

        Ok(())
    }
}
