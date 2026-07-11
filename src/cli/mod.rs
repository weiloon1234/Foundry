use std::sync::Arc;

use clap::{ArgMatches, Command};

use crate::foundation::{AppContext, Error, Result};
use crate::logging::{catch_sync_panic, panic_payload_message};
use crate::support::CommandId;
use crate::support::{boxed, BoxFuture};

pub(crate) mod dev;
mod io;

pub use io::{CommandIo, CommandProgress, TerminalCommandIo};

pub type CommandRegistrar = Arc<dyn Fn(&mut CommandRegistry) -> Result<()> + Send + Sync>;
type CommandHandler =
    Arc<dyn Fn(CommandInvocation) -> BoxFuture<Result<CommandExit>> + Send + Sync>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandExit(u8);

impl CommandExit {
    pub const SUCCESS: Self = Self(0);
    pub const FAILURE: Self = Self(1);

    pub const fn new(code: u8) -> Self {
        Self(code)
    }

    pub const fn code(self) -> u8 {
        self.0
    }

    pub const fn is_success(self) -> bool {
        self.0 == 0
    }

    pub(crate) fn into_result(self) -> Result<()> {
        if self.is_success() {
            Ok(())
        } else {
            Err(Error::message(format!(
                "command exited with status {}",
                self.code()
            )))
        }
    }
}

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
    io: Arc<dyn CommandIo>,
}

impl CommandInvocation {
    pub(crate) fn new(app: AppContext, matches: ArgMatches, io: Arc<dyn CommandIo>) -> Self {
        Self { app, matches, io }
    }

    pub fn app(&self) -> &AppContext {
        &self.app
    }

    pub fn matches(&self) -> &ArgMatches {
        &self.matches
    }

    pub fn io(&self) -> &dyn CommandIo {
        self.io.as_ref()
    }

    pub fn write(&self, message: impl AsRef<str>) -> Result<()> {
        io::stdout(&self.io, message.as_ref())
    }

    pub fn line(&self, message: impl AsRef<str>) -> Result<()> {
        self.write(format!("{}\n", message.as_ref()))
    }

    pub fn error(&self, message: impl AsRef<str>) -> Result<()> {
        io::stderr(&self.io, &format!("{}\n", message.as_ref()))
    }

    pub fn prompt(&self, question: impl AsRef<str>) -> Result<String> {
        self.write(format!("{}: ", question.as_ref()))?;
        let value = self.io.read_stdin_line().map_err(Error::other)?;
        Ok(value.trim_end_matches(['\r', '\n']).to_string())
    }

    pub fn confirm(&self, question: impl AsRef<str>, default: bool) -> Result<bool> {
        let question = question.as_ref();
        let suffix = if default { "[Y/n]" } else { "[y/N]" };
        loop {
            let answer = self.prompt(format!("{question} {suffix}"))?;
            match answer.trim().to_ascii_lowercase().as_str() {
                "" => return Ok(default),
                "y" | "yes" => return Ok(true),
                "n" | "no" => return Ok(false),
                _ => self.error("Please answer yes or no.")?,
            }
        }
    }

    pub fn progress(&self, label: impl Into<String>, total: u64) -> Result<CommandProgress> {
        CommandProgress::start(self.io.clone(), label, total)
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
        self.register_handler(
            id.into(),
            command,
            Arc::new(move |invocation| {
                let future = handler(invocation);
                boxed(async move { future.await.map(|()| CommandExit::SUCCESS) })
            }),
        )
    }

    pub fn command_with_exit<I, F, Fut>(
        &mut self,
        id: I,
        command: Command,
        handler: F,
    ) -> Result<&mut Self>
    where
        I: Into<CommandId>,
        F: Fn(CommandInvocation) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<CommandExit>> + Send + 'static,
    {
        self.register_handler(
            id.into(),
            command,
            Arc::new(move |invocation| boxed(handler(invocation))),
        )
    }

    fn register_handler(
        &mut self,
        id: CommandId,
        command: Command,
        handler: CommandHandler,
    ) -> Result<&mut Self> {
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
            handler,
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
