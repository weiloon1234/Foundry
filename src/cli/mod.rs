use std::sync::Arc;

use clap::{Arg, ArgMatches, Command};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandDescriptor {
    pub id: CommandId,
    pub name: String,
    pub about: Option<String>,
    pub long_about: Option<String>,
    pub arguments: Vec<String>,
    pub argument_metadata: Vec<CommandArgumentMetadata>,
    pub positional_arguments: Vec<String>,
    pub options: Vec<String>,
    pub value_options: Vec<String>,
    pub flags: Vec<String>,
    pub option_switches: Vec<CommandOptionSwitchDescriptor>,
    pub subcommands: Vec<String>,
    pub arg_count: usize,
    pub positional_arg_count: usize,
    pub option_count: usize,
    pub value_option_count: usize,
    pub flag_count: usize,
    pub subcommand_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandArgumentMetadata {
    pub name: String,
    pub kind: CommandArgumentKind,
    pub help: Option<String>,
    pub long_help: Option<String>,
    pub required: bool,
    pub repeatable: bool,
    pub value_names: Vec<String>,
    pub value_hint: Option<String>,
    pub default_values: Vec<String>,
    pub possible_values: Vec<CommandArgumentPossibleValue>,
}

impl CommandArgumentMetadata {
    fn from_arg(name: String, arg: &Arg) -> Self {
        let kind = if arg.is_positional() {
            CommandArgumentKind::Positional
        } else if arg.get_action().takes_values() {
            CommandArgumentKind::ValueOption
        } else {
            CommandArgumentKind::Flag
        };
        let default_values = arg
            .get_default_values()
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        let value_names = arg
            .get_value_names()
            .unwrap_or_default()
            .iter()
            .map(|name| name.as_str().to_string())
            .collect();
        let possible_values = arg
            .get_possible_values()
            .into_iter()
            .map(|value| CommandArgumentPossibleValue {
                name: value.get_name().to_string(),
                help: value.get_help().map(|help| help.to_string()),
                hidden: value.is_hide_set(),
            })
            .collect();

        Self {
            name,
            kind,
            help: arg.get_help().map(|help| help.to_string()),
            long_help: arg.get_long_help().map(|help| help.to_string()),
            required: arg.is_required_set(),
            repeatable: command_argument_accepts_repeated_input(arg),
            value_names,
            value_hint: command_argument_value_hint(arg),
            default_values,
            possible_values,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandArgumentPossibleValue {
    pub name: String,
    pub help: Option<String>,
    pub hidden: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandArgumentKind {
    Positional,
    ValueOption,
    Flag,
}

impl CommandArgumentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Positional => "positional",
            Self::ValueOption => "value_option",
            Self::Flag => "flag",
        }
    }
}

fn command_argument_accepts_repeated_input(arg: &Arg) -> bool {
    matches!(
        arg.get_action(),
        clap::ArgAction::Append | clap::ArgAction::Count
    ) || arg
        .get_num_args()
        .is_some_and(|range| range.max_values() > 1)
}

fn command_argument_value_hint(arg: &Arg) -> Option<String> {
    let hint = match arg.get_value_hint() {
        clap::ValueHint::Unknown => return None,
        clap::ValueHint::Other => "other",
        clap::ValueHint::AnyPath => "any_path",
        clap::ValueHint::FilePath => "file_path",
        clap::ValueHint::DirPath => "dir_path",
        clap::ValueHint::ExecutablePath => "executable_path",
        clap::ValueHint::CommandName => "command_name",
        clap::ValueHint::CommandString => "command_string",
        clap::ValueHint::CommandWithArguments => "command_with_arguments",
        clap::ValueHint::Username => "username",
        clap::ValueHint::Hostname => "hostname",
        clap::ValueHint::Url => "url",
        clap::ValueHint::EmailAddress => "email_address",
        _ => "unknown",
    };
    Some(hint.to_string())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOptionSwitchDescriptor {
    pub name: String,
    pub short: Option<String>,
    pub long: Option<String>,
    pub tokens: Vec<String>,
}

impl CommandOptionSwitchDescriptor {
    fn from_arg(name: String, arg: &Arg) -> Self {
        let short = arg.get_short().map(|short| short.to_string());
        let long = arg.get_long().map(str::to_string);
        let mut tokens = Vec::new();
        if let Some(long) = &long {
            tokens.push(format!("--{long}"));
        }
        if let Some(short) = &short {
            tokens.push(format!("-{short}"));
        }

        Self {
            name,
            short,
            long,
            tokens,
        }
    }
}

impl RegisteredCommand {
    fn descriptor(&self) -> CommandDescriptor {
        let mut arguments = Vec::new();
        let mut argument_metadata = Vec::new();
        let mut positional_arguments = Vec::new();
        let mut options = Vec::new();
        let mut value_options = Vec::new();
        let mut flags = Vec::new();
        let mut option_switches = Vec::new();
        for arg in self.command.get_arguments() {
            let name = arg.get_id().as_str().to_string();
            argument_metadata.push(CommandArgumentMetadata::from_arg(name.clone(), arg));
            if arg.is_positional() {
                positional_arguments.push(name.clone());
            } else {
                option_switches.push(CommandOptionSwitchDescriptor::from_arg(name.clone(), arg));
                if arg.get_action().takes_values() {
                    value_options.push(name.clone());
                } else {
                    flags.push(name.clone());
                }
                options.push(name.clone());
            }
            arguments.push(name);
        }
        let subcommands = self
            .command
            .get_subcommands()
            .map(|command| command.get_name().to_string())
            .collect::<Vec<_>>();
        let arg_count = arguments.len();
        let positional_arg_count = positional_arguments.len();
        let option_count = options.len();
        let value_option_count = value_options.len();
        let flag_count = flags.len();
        let subcommand_count = subcommands.len();

        CommandDescriptor {
            id: self.id.clone(),
            name: self.command.get_name().to_string(),
            about: self.command.get_about().map(|about| about.to_string()),
            long_about: self.command.get_long_about().map(|about| about.to_string()),
            arguments,
            argument_metadata,
            positional_arguments,
            options,
            value_options,
            flags,
            option_switches,
            subcommands,
            arg_count,
            positional_arg_count,
            option_count,
            value_option_count,
            flag_count,
            subcommand_count,
        }
    }
}

#[derive(Clone)]
pub struct CommandInvocation {
    app: AppContext,
    matches: ArgMatches,
    commands: Vec<CommandDescriptor>,
}

impl CommandInvocation {
    pub(crate) fn new(
        app: AppContext,
        matches: ArgMatches,
        commands: Vec<CommandDescriptor>,
    ) -> Self {
        Self {
            app,
            matches,
            commands,
        }
    }

    pub fn app(&self) -> &AppContext {
        &self.app
    }

    pub fn matches(&self) -> &ArgMatches {
        &self.matches
    }

    pub fn commands(&self) -> &[CommandDescriptor] {
        &self.commands
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

    pub fn descriptors(&self) -> Vec<CommandDescriptor> {
        let mut descriptors = self
            .commands
            .iter()
            .map(RegisteredCommand::descriptor)
            .collect::<Vec<_>>();
        descriptors.sort_by(|a, b| a.id.cmp(&b.id));
        descriptors
    }
}

#[cfg(test)]
mod tests {
    use clap::{builder::PossibleValue, Arg, ArgAction, Command, ValueHint};

    use super::{
        CommandArgumentKind, CommandArgumentMetadata, CommandArgumentPossibleValue,
        CommandOptionSwitchDescriptor, CommandRegistry,
    };
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

    #[test]
    fn descriptors_expose_registered_command_metadata() {
        let mut registry = CommandRegistry::new();
        registry
            .command(
                CommandId::new("reports:export"),
                Command::new("reports:export")
                    .about("Export reports")
                    .long_about("Export reports to a configured destination")
                    .arg(
                        Arg::new("date")
                            .required(true)
                            .value_name("DATE")
                            .help("Report date"),
                    )
                    .arg(
                        Arg::new("format")
                            .short('f')
                            .long("format")
                            .value_name("FORMAT")
                            .help("Output format")
                            .long_help("Output format for generated report files")
                            .value_hint(ValueHint::Other)
                            .value_parser([
                                PossibleValue::new("json").help("JSON lines"),
                                PossibleValue::new("csv").help("CSV file"),
                                PossibleValue::new("internal").hide(true),
                            ])
                            .default_value("json")
                            .action(ArgAction::Append),
                    )
                    .arg(
                        Arg::new("dry_run")
                            .short('d')
                            .long("dry-run")
                            .help("Preview without writing files")
                            .action(ArgAction::SetTrue),
                    )
                    .subcommand(Command::new("daily")),
                |_invocation| async { Ok(()) },
            )
            .unwrap();

        let descriptors = registry.descriptors();

        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].id, CommandId::new("reports:export"));
        assert_eq!(descriptors[0].name, "reports:export");
        assert_eq!(descriptors[0].about.as_deref(), Some("Export reports"));
        assert_eq!(
            descriptors[0].long_about.as_deref(),
            Some("Export reports to a configured destination")
        );
        assert_eq!(descriptors[0].arguments, vec!["date", "format", "dry_run"]);
        assert_eq!(
            descriptors[0].argument_metadata,
            vec![
                CommandArgumentMetadata {
                    name: "date".to_string(),
                    kind: CommandArgumentKind::Positional,
                    help: Some("Report date".to_string()),
                    long_help: None,
                    required: true,
                    repeatable: false,
                    value_names: vec!["DATE".to_string()],
                    value_hint: None,
                    default_values: Vec::new(),
                    possible_values: Vec::new(),
                },
                CommandArgumentMetadata {
                    name: "format".to_string(),
                    kind: CommandArgumentKind::ValueOption,
                    help: Some("Output format".to_string()),
                    long_help: Some("Output format for generated report files".to_string()),
                    required: false,
                    repeatable: true,
                    value_names: vec!["FORMAT".to_string()],
                    value_hint: Some("other".to_string()),
                    default_values: vec!["json".to_string()],
                    possible_values: vec![
                        CommandArgumentPossibleValue {
                            name: "json".to_string(),
                            help: Some("JSON lines".to_string()),
                            hidden: false,
                        },
                        CommandArgumentPossibleValue {
                            name: "csv".to_string(),
                            help: Some("CSV file".to_string()),
                            hidden: false,
                        },
                        CommandArgumentPossibleValue {
                            name: "internal".to_string(),
                            help: None,
                            hidden: true,
                        },
                    ],
                },
                CommandArgumentMetadata {
                    name: "dry_run".to_string(),
                    kind: CommandArgumentKind::Flag,
                    help: Some("Preview without writing files".to_string()),
                    long_help: None,
                    required: false,
                    repeatable: false,
                    value_names: Vec::new(),
                    value_hint: None,
                    default_values: Vec::new(),
                    possible_values: Vec::new(),
                },
            ]
        );
        assert_eq!(descriptors[0].positional_arguments, vec!["date"]);
        assert_eq!(descriptors[0].options, vec!["format", "dry_run"]);
        assert_eq!(descriptors[0].value_options, vec!["format"]);
        assert_eq!(descriptors[0].flags, vec!["dry_run"]);
        assert_eq!(
            descriptors[0].option_switches,
            vec![
                CommandOptionSwitchDescriptor {
                    name: "format".to_string(),
                    short: Some("f".to_string()),
                    long: Some("format".to_string()),
                    tokens: vec!["--format".to_string(), "-f".to_string()],
                },
                CommandOptionSwitchDescriptor {
                    name: "dry_run".to_string(),
                    short: Some("d".to_string()),
                    long: Some("dry-run".to_string()),
                    tokens: vec!["--dry-run".to_string(), "-d".to_string()],
                },
            ]
        );
        assert_eq!(descriptors[0].subcommands, vec!["daily"]);
        assert_eq!(descriptors[0].arg_count, 3);
        assert_eq!(descriptors[0].positional_arg_count, 1);
        assert_eq!(descriptors[0].option_count, 2);
        assert_eq!(descriptors[0].value_option_count, 1);
        assert_eq!(descriptors[0].flag_count, 1);
        assert_eq!(descriptors[0].subcommand_count, 1);
    }
}
