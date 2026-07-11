use std::ffi::OsString;

use clap::{error::ErrorKind, Command};

use std::sync::Arc;

use crate::cli::{CommandExit, CommandInvocation, CommandIo, CommandRegistry, TerminalCommandIo};
use crate::foundation::{AppContext, Error, Result};
use crate::logging::{catch_future_panic, panic_payload_message};

pub struct CliKernel {
    app: AppContext,
    registrars: Vec<crate::cli::CommandRegistrar>,
    io: Arc<dyn CommandIo>,
}

impl CliKernel {
    pub fn new(app: AppContext, registrars: Vec<crate::cli::CommandRegistrar>) -> Self {
        Self {
            app,
            registrars,
            io: Arc::new(TerminalCommandIo),
        }
    }

    pub fn with_io<I>(mut self, io: I) -> Self
    where
        I: CommandIo,
    {
        self.io = Arc::new(io);
        self
    }

    pub fn build_registry(&self) -> Result<CommandRegistry> {
        crate::cli::build_registry(&self.registrars)
    }

    pub fn app(&self) -> &AppContext {
        &self.app
    }

    pub async fn run(self) -> Result<()> {
        self.run_status().await?.into_result()
    }

    pub async fn run_status(self) -> Result<CommandExit> {
        self.run_with_args_status(std::env::args_os()).await
    }

    pub async fn run_with_args<I, T>(self, args: I) -> Result<()>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        self.run_with_args_status(args).await?.into_result()
    }

    pub async fn run_with_args_status<I, T>(self, args: I) -> Result<CommandExit>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let registry = self.build_registry()?;
        let mut root = Command::new("foundry")
            .version(env!("CARGO_PKG_VERSION"))
            .about("Foundry — a Laravel-inspired Rust web framework")
            .subcommand_required(true)
            .arg_required_else_help(true);
        for registered in registry.commands() {
            root = root.subcommand(registered.command.clone());
        }

        let matches = match root.try_get_matches_from(args) {
            Ok(matches) => matches,
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
                ) =>
            {
                self.io
                    .write_stdout(&error.to_string())
                    .map_err(Error::other)?;
                return Ok(CommandExit::SUCCESS);
            }
            Err(error) => return Err(Error::other(error)),
        };
        if let Some((name, sub_matches)) = matches.subcommand() {
            if let Some(registered) = registry
                .commands()
                .iter()
                .find(|command| command.id.as_str() == name)
            {
                let handler = registered.handler.clone();
                let invocation =
                    CommandInvocation::new(self.app.clone(), sub_matches.clone(), self.io.clone());
                match catch_future_panic(async move { handler(invocation).await }).await {
                    Ok(result) => return result,
                    Err(panic) => {
                        let message = panic_payload_message(panic);
                        tracing::error!(
                            command = %registered.id,
                            panic = %message,
                            "CLI command panicked"
                        );
                        return Err(Error::message(format!("cli command panicked: {message}")));
                    }
                }
            }
        }

        Ok(CommandExit::SUCCESS)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use clap::Command;

    use super::CliKernel;
    use crate::cli::{CommandExit, CommandRegistrar};
    use crate::config::ConfigRepository;
    use crate::foundation::{AppContext, Container, Error};
    use crate::support::CommandId;
    use crate::testing::CommandIoFake;
    use crate::validation::RuleRegistry;

    fn test_app() -> AppContext {
        AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap()
    }

    fn kernel_with_registrar(registrar: CommandRegistrar) -> CliKernel {
        CliKernel::new(test_app(), vec![registrar])
    }

    #[tokio::test]
    async fn command_success_runs_normally() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let command_calls = calls.clone();
        let registrar: CommandRegistrar = Arc::new(move |registry| {
            let command_calls = command_calls.clone();
            registry.command(
                CommandId::new("hello"),
                Command::new("hello"),
                move |_invocation| {
                    let command_calls = command_calls.clone();
                    async move {
                        command_calls.lock().unwrap().push("hello");
                        Ok(())
                    }
                },
            )?;
            Ok(())
        });

        kernel_with_registrar(registrar)
            .run_with_args(["foundry", "hello"])
            .await
            .unwrap();

        assert_eq!(calls.lock().unwrap().as_slice(), ["hello"]);
    }

    #[tokio::test]
    async fn command_error_returns_existing_error() {
        let registrar: CommandRegistrar = Arc::new(|registry| {
            registry.command(
                CommandId::new("fail"),
                Command::new("fail"),
                |_invocation| async { Err(Error::message("command failed")) },
            )?;
            Ok(())
        });

        let error = kernel_with_registrar(registrar)
            .run_with_args(["foundry", "fail"])
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "command failed");
    }

    #[tokio::test]
    async fn root_help_and_version_are_successful_cli_outcomes() {
        let registrar: CommandRegistrar = Arc::new(|registry| {
            registry.command(
                CommandId::new("hello"),
                Command::new("hello"),
                |_invocation| async { Ok(()) },
            )?;
            Ok(())
        });

        let help = CommandIoFake::new();
        kernel_with_registrar(registrar.clone())
            .with_io(help.clone())
            .run_with_args(["foundry", "--help"])
            .await
            .unwrap();
        help.assert_stdout_contains("Usage:");

        let version = CommandIoFake::new();
        kernel_with_registrar(registrar)
            .with_io(version.clone())
            .run_with_args(["foundry", "--version"])
            .await
            .unwrap();
        version.assert_stdout_contains(env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn injected_io_captures_prompts_progress_and_typed_exit_status() {
        let registrar: CommandRegistrar = Arc::new(|registry| {
            registry.command_with_exit(
                CommandId::new("interactive"),
                Command::new("interactive"),
                |invocation| async move {
                    invocation.line("starting")?;
                    if !invocation.confirm("Continue", false)? {
                        return Ok(CommandExit::new(4));
                    }

                    let mut progress = invocation.progress("Import", 2)?;
                    progress.advance(1)?;
                    progress.finish()?;
                    invocation.error("diagnostic")?;
                    Ok(CommandExit::new(7))
                },
            )?;
            Ok(())
        });
        let io = CommandIoFake::new()
            .with_input("not-an-answer")
            .with_input("yes");

        let status = kernel_with_registrar(registrar)
            .with_io(io.clone())
            .run_with_args_status(["foundry", "interactive"])
            .await
            .unwrap();

        assert_eq!(status.code(), 7);
        io.assert_stdout_contains("starting\n")
            .assert_stdout_contains("Continue [y/N]: ")
            .assert_stdout_contains("Import: 0/2\n")
            .assert_stdout_contains("Import: 1/2\n")
            .assert_stdout_contains("Import: 2/2\n")
            .assert_stderr("Please answer yes or no.\ndiagnostic\n");
    }

    #[test]
    fn command_registrar_panic_becomes_error() {
        let registrar: CommandRegistrar = Arc::new(|_| {
            panic!("command registrar explode");
        });

        let error = match kernel_with_registrar(registrar).build_registry() {
            Ok(_) => panic!("expected command registrar panic error"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "cli registrar panicked: command registrar explode"
        );
    }

    #[tokio::test]
    async fn command_future_panic_becomes_error() {
        let registrar: CommandRegistrar = Arc::new(|registry| {
            registry.command(
                CommandId::new("panic"),
                Command::new("panic"),
                |_invocation| async {
                    panic!("command explode");
                    #[allow(unreachable_code)]
                    Ok(())
                },
            )?;
            Ok(())
        });

        let error = kernel_with_registrar(registrar)
            .run_with_args(["foundry", "panic"])
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "cli command panicked: command explode");
    }

    #[tokio::test]
    async fn command_factory_panic_becomes_error() {
        let registrar: CommandRegistrar = Arc::new(|registry| {
            registry.command(
                CommandId::new("panic-build"),
                Command::new("panic-build"),
                |_invocation| -> std::future::Ready<crate::Result<()>> {
                    panic!("command build explode")
                },
            )?;
            Ok(())
        });

        let error = kernel_with_registrar(registrar)
            .run_with_args(["foundry", "panic-build"])
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "cli command panicked: command build explode"
        );
    }
}
