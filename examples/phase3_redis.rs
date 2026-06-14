use foundry::prelude::*;

const REDIS_DEMO: CommandId = CommandId::new("redis-demo");

fn main() -> Result<()> {
    App::builder()
        .load_env()
        .load_config_dir("config")
        .register_commands(register_commands)
        .run_cli()
}

fn register_commands(registry: &mut CommandRegistry) -> Result<()> {
    registry.command(
        REDIS_DEMO,
        Command::new("redis-demo"),
        |invocation| async move {
            let redis = invocation.app().redis()?;
            let session_key = redis.key("sessions:user-123");
            let analytics_key = redis.key_in_namespace("analytics:prod", "daily:users");
            let events_channel = redis.channel("events:users");

            let mut connection = redis.connection().await?;
            connection.set_ex(&session_key, "active", 300).await?;
            let session: String = connection.get(&session_key).await?;
            let analytics_exists = connection.exists(&analytics_key).await?;
            connection
                .publish(&events_channel, format!("session={session}"))
                .await?;

            println!(
                "session={}, analytics_exists={analytics_exists}, channel={}",
                session, events_channel
            );

            Ok(())
        },
    )?;

    Ok(())
}
