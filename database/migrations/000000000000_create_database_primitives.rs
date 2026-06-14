use async_trait::async_trait;
use foundry::prelude::*;

pub struct Entry;

#[async_trait]
impl MigrationFile for Entry {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute("CREATE EXTENSION IF NOT EXISTS pgcrypto", &[])
            .await?;

        ctx.raw_execute(
            r#"
            DO $foundry$
            BEGIN
                IF NOT EXISTS (
                    SELECT 1
                    FROM pg_proc p
                    JOIN pg_namespace n ON n.oid = p.pronamespace
                    WHERE n.nspname = 'public'
                      AND p.proname = 'uuidv7'
                      AND pg_get_function_identity_arguments(p.oid) = ''
                ) THEN
                    EXECUTE $function$
                        CREATE FUNCTION public.uuidv7()
                        RETURNS uuid
                        LANGUAGE sql
                        VOLATILE
                        AS $uuidv7$
                            WITH value AS (
                                SELECT
                                    (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint AS unix_ts_ms,
                                    gen_random_bytes(10) AS rand_bytes
                            )
                            SELECT encode(
                                decode(lpad(to_hex(unix_ts_ms), 12, '0'), 'hex')
                                || set_byte(
                                    substring(rand_bytes from 1 for 2),
                                    0,
                                    (get_byte(rand_bytes, 0) & 15) | 112
                                )
                                || set_byte(
                                    substring(rand_bytes from 3 for 8),
                                    0,
                                    (get_byte(rand_bytes, 2) & 63) | 128
                                ),
                                'hex'
                            )::uuid
                            FROM value
                        $uuidv7$;
                    $function$;
                END IF;
            END
            $foundry$;
            "#,
            &[],
        )
        .await?;

        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            r#"
            DO $foundry$
            BEGIN
                IF EXISTS (
                    SELECT 1
                    FROM pg_proc p
                    JOIN pg_namespace n ON n.oid = p.pronamespace
                    WHERE n.nspname = 'public'
                      AND p.proname = 'uuidv7'
                      AND pg_get_function_identity_arguments(p.oid) = ''
                      AND pg_get_userbyid(p.proowner) = current_user
                ) THEN
                    DROP FUNCTION public.uuidv7();
                END IF;
            END
            $foundry$;
            "#,
            &[],
        )
        .await?;
        Ok(())
    }
}
