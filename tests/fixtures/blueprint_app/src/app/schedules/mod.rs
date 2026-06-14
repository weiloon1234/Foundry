use foundry::prelude::*;

use crate::app::ids;

pub fn register(registry: &mut ScheduleRegistry) -> Result<()> {
    registry.cron(
        ids::HEARTBEAT_SCHEDULE,
        CronExpression::parse("*/1 * * * * *")?,
        |invocation| async move {
            let entries = invocation.app().resolve::<std::sync::Mutex<Vec<String>>>()?;
            entries
                .lock()
                .unwrap()
                .push("schedule:heartbeat".to_string());
            Ok(())
        },
    )?;
    Ok(())
}
