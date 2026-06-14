use foundry::prelude::*;

use crate::app::ids;

pub fn register(registrar: &mut WebSocketRegistrar) -> Result<()> {
    registrar.channel_with_options(
        ids::CHAT_CHANNEL,
        |context: WebSocketContext, payload: serde_json::Value| async move {
            context.publish(ids::ECHO_EVENT, payload).await
        },
        WebSocketChannelOptions::new()
            .guard(ids::AuthGuard::Api)
            .permission(ids::Ability::RealtimeChat),
    )?;
    Ok(())
}
