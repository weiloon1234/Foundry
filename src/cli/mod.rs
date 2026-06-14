use std::sync::Arc;

use clap::{ArgMatches, Command};

use crate::foundation::{AppContext, Error, Result};
use crate::logging::{catch_sync_panic, panic_payload_message};
use crate::support::CommandId;
use crate::support::{boxed, BoxFuture};

pub type CommandRegistrar = Arc<dyn Fn(&mut CommandRegistry) -> Result<()> + Send + Sync>;
type CommandHandler = Arc<dyn Fn(CommandInvocation) -> BoxFuture<Result<()>> + Send + Sync>;

pub(crate) fn build_registry(registrars: &[CommandRegistrar]) -> Result<CommandRegistry> {
    let mut registry = CommandRegistry::new();
    for registrar in registrars {
        match catch_sync_panic(|| registrar(&mut registry)) {
            Ok(result) => result?,
            Err(panic) => return Err(command_registrar_panic_error(panic)),
        }
    }
    Ok(registry)
}

fn command_registrar_panic_error(panic: Box<dyn std::any::Any + Send>) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.cli",
        panic = %message,
        "CLI registrar panicked"
    );
    Error::message(format!("cli registrar panicked: {message}"))
}

pub struct RegisteredCommand {
    pub(crate) id: CommandId,
    pub(crate) command: Command,
    pub(crate) handler: CommandHandler,
}

#[derive(Clone)]
pub struct CommandInvocation {
    app: AppContext,
    matches: ArgMatches,
}

impl CommandInvocation {
    pub(crate) fn new(app: AppContext, matches: ArgMatches) -> Self {
        Self { app, matches }
    }

    pub fn app(&self) -> &AppContext {
        &self.app
    }

    pub fn matches(&self) -> &ArgMatches {
        &self.matches
    }
}

#[derive(Default)]
pub struct CommandRegistry {
    commands: Vec<RegisteredCommand>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn command<I, F, Fut>(&mut self, id: I, command: Command, handler: F) -> Result<&mut Self>
    where
        I: Into<CommandId>,
        F: Fn(CommandInvocation) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        let id = id.into();
        if command.get_name() != id.as_str() {
            return Err(Error::message(format!(
                "command id `{id}` must match clap command name `{}`",
                command.get_name()
            )));
        }
        if self.commands.iter().any(|registered| registered.id == id) {
            return Err(Error::message(format!("command `{id}` already registered")));
        }

        self.commands.push(RegisteredCommand {
            id,
            command,
            handler: Arc::new(move |invocation| boxed(handler(invocation))),
        });
        Ok(self)
    }

    pub(crate) fn commands(&self) -> &[RegisteredCommand] {
        &self.commands
    }
}

#[cfg(test)]
mod tests {
    use clap::Command;

    use super::CommandRegistry;
    use crate::support::CommandId;

    #[test]
    fn rejects_duplicate_command_names() {
        let mut registry = CommandRegistry::new();
        registry
            .command(
                CommandId::new("hello"),
                Command::new("hello"),
                |_invocation| async { Ok(()) },
            )
            .unwrap();

        let error = registry
            .command(
                CommandId::new("hello"),
                Command::new("hello"),
                |_invocation| async { Ok(()) },
            )
            .err()
            .unwrap();
        assert!(error.to_string().contains("already registered"));
    }
}
