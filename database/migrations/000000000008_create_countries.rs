use async_trait::async_trait;
use foundry::prelude::*;

pub struct Entry;

#[async_trait]
impl MigrationFile for Entry {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            r#"
            CREATE TABLE countries (
                iso2 CHAR(2) PRIMARY KEY,
                iso3 CHAR(3) NOT NULL,
                iso_numeric TEXT,
                name TEXT NOT NULL,
                official_name TEXT,
                capital TEXT,
                region TEXT,
                subregion TEXT,
                currencies JSONB NOT NULL DEFAULT '[]',
                primary_currency_code TEXT,
                calling_code TEXT,
                calling_root TEXT,
                calling_suffixes JSONB NOT NULL DEFAULT '[]',
                tlds JSONB NOT NULL DEFAULT '[]',
                timezones JSONB NOT NULL DEFAULT '[]',
                latitude DOUBLE PRECISION,
                longitude DOUBLE PRECISION,
                independent BOOLEAN,
                un_member BOOLEAN,
                flag_emoji TEXT,
                conversion_rate DOUBLE PRECISION,
                is_default BOOLEAN NOT NULL DEFAULT false,
                status TEXT NOT NULL DEFAULT 'disabled',
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ
            )
            "#,
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_countries_status ON countries (status)",
            &[],
        )
        .await?;

        ctx.raw_execute(
            "CREATE INDEX idx_countries_region ON countries (region)",
            &[],
        )
        .await?;

        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute("DROP TABLE IF EXISTS countries", &[])
            .await?;
        Ok(())
    }
}
