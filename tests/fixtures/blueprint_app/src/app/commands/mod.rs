use foundry::prelude::*;

use crate::app::ids;

pub fn register(registry: &mut CommandRegistry) -> Result<()> {
    registry.command(
        ids::PING_COMMAND,
        Command::new("ping").about("Blueprint fixture command"),
        |invocation: CommandInvocation| async move {
            let entries = invocation.app().resolve::<std::sync::Mutex<Vec<String>>>()?;
            entries.lock().unwrap().push("command:ping".to_string());
            Ok(())
        },
    )?;
    Ok(())
}
