use std::sync::Arc;

use clap::{Arg, Command};

use crate::cli::{CommandRegistrar, CommandRegistry};
use crate::foundation::{Error, Result};
use crate::support::CommandId;

const AUDIT_PRUNE_COMMAND: CommandId = CommandId::new("audit:prune");

pub(crate) fn audit_cli_registrar() -> CommandRegistrar {
    Arc::new(register)
}

fn register(registry: &mut CommandRegistry) -> Result<()> {
    registry.command(
        AUDIT_PRUNE_COMMAND,
        Command::new(AUDIT_PRUNE_COMMAND.as_str().to_string())
            .about("Delete audit rows older than the configured retention window")
            .arg(
                Arg::new("days")
                    .long("days")
                    .value_name("DAYS")
                    .value_parser(clap::value_parser!(u32))
                    .help("Override audit.retention_days for this run"),
            ),
        |invocation| async move {
            let configured_days = invocation.app().audit()?.retention_days();
            let days = invocation
                .matches()
                .get_one::<u32>("days")
                .copied()
                .unwrap_or(configured_days);
            if days == 0 {
                return Err(Error::message(
                    "audit retention is disabled; configure audit.retention_days or pass --days",
                ));
            }

            let cutoff = invocation.app().clock().now().sub_days(i64::from(days));
            let database = invocation.app().database()?;
            let deleted = invocation
                .app()
                .audit()?
                .prune_before(database.as_ref(), cutoff)
                .await?;
            invocation.line(format!(
                "pruned {deleted} audit row(s) older than {days} day(s)"
            ))?;
            Ok(())
        },
    )?;
    Ok(())
}
