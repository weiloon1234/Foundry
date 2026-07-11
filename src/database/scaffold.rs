use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Local;
use clap::{Arg, ArgAction, ArgMatches, Command};

use crate::cli::{CommandInvocation, CommandRegistrar, CommandRegistry};
use crate::config::DatabaseConfig;
use crate::database::lifecycle::GeneratedDatabasePaths;
use crate::foundation::{AppContext, Error, Result};
use crate::support::generated_manifest::{ensure_generated_file_writable, write_generated_file};
use crate::support::CommandId;

const MAKE_MIGRATION_COMMAND: CommandId = CommandId::new("make:migration");
const MAKE_SEEDER_COMMAND: CommandId = CommandId::new("make:seeder");
const MAKE_MODEL_COMMAND: CommandId = CommandId::new("make:model");
const MAKE_JOB_COMMAND: CommandId = CommandId::new("make:job");
const MAKE_COMMAND_COMMAND: CommandId = CommandId::new("make:command");
const MAKE_REQUEST_COMMAND: CommandId = CommandId::new("make:request");
const MAKE_DTO_COMMAND: CommandId = CommandId::new("make:dto");
const MAKE_POLICY_COMMAND: CommandId = CommandId::new("make:policy");
const MAKE_EVENT_COMMAND: CommandId = CommandId::new("make:event");
const MAKE_LISTENER_COMMAND: CommandId = CommandId::new("make:listener");
const MAKE_NOTIFICATION_COMMAND: CommandId = CommandId::new("make:notification");
const MAKE_MAIL_COMMAND: CommandId = CommandId::new("make:mail");
const MAKE_DATATABLE_COMMAND: CommandId = CommandId::new("make:datatable");
const MAKE_PLUGIN_COMMAND: CommandId = CommandId::new("make:plugin");
const MAKE_TEST_COMMAND: CommandId = CommandId::new("make:test");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComponentScaffold {
    Request,
    Dto,
    Policy,
    Event,
    Listener,
    Notification,
    Mail,
    Datatable,
    Plugin,
    Test,
}

impl ComponentScaffold {
    const ALL: [Self; 10] = [
        Self::Request,
        Self::Dto,
        Self::Policy,
        Self::Event,
        Self::Listener,
        Self::Notification,
        Self::Mail,
        Self::Datatable,
        Self::Plugin,
        Self::Test,
    ];

    const fn command_id(self) -> CommandId {
        match self {
            Self::Request => MAKE_REQUEST_COMMAND,
            Self::Dto => MAKE_DTO_COMMAND,
            Self::Policy => MAKE_POLICY_COMMAND,
            Self::Event => MAKE_EVENT_COMMAND,
            Self::Listener => MAKE_LISTENER_COMMAND,
            Self::Notification => MAKE_NOTIFICATION_COMMAND,
            Self::Mail => MAKE_MAIL_COMMAND,
            Self::Datatable => MAKE_DATATABLE_COMMAND,
            Self::Plugin => MAKE_PLUGIN_COMMAND,
            Self::Test => MAKE_TEST_COMMAND,
        }
    }

    const fn default_path(self) -> &'static str {
        match self {
            Self::Request => "src/app/requests",
            Self::Dto => "src/app/dtos",
            Self::Policy => "src/app/policies",
            Self::Event => "src/app/events",
            Self::Listener => "src/app/listeners",
            Self::Notification => "src/app/notifications",
            Self::Mail => "src/app/mail",
            Self::Datatable => "src/app/datatables",
            Self::Plugin => "src/app/plugins",
            Self::Test => "tests",
        }
    }

    const fn about(self) -> &'static str {
        match self {
            Self::Request => "Generate a validated HTTP request DTO component",
            Self::Dto => "Generate a serializable API DTO component",
            Self::Policy => "Generate an authorization policy component",
            Self::Event => "Generate a domain event component",
            Self::Listener => "Generate an event listener component",
            Self::Notification => "Generate a notification component",
            Self::Mail => "Generate an email message component",
            Self::Datatable => "Generate a model datatable component",
            Self::Plugin => "Generate an in-app plugin component",
            Self::Test => "Generate an integration test component",
        }
    }

    const fn next_step(self) -> &'static str {
        match self {
            Self::Request | Self::Dto => {
                "next: define fields, then reference the DTO from route documentation"
            }
            Self::Policy => "next: implement evaluate(), then register the policy",
            Self::Event => "next: add event data, then register one or more listeners",
            Self::Listener => "next: adjust the event import, implement handle(), and register it",
            Self::Notification => "next: select delivery channels and implement their payloads",
            Self::Mail => "next: customize subject/body and call build() from your mail flow",
            Self::Datatable => "next: adjust the model import and declare columns/filters",
            Self::Plugin => "next: add plugin contributions, then register the plugin",
            Self::Test => "next: register the app modules needed by this test",
        }
    }
}

pub(crate) fn scaffold_cli_registrar() -> CommandRegistrar {
    Arc::new(register_cli_commands)
}

fn register_cli_commands(registry: &mut CommandRegistry) -> Result<()> {
    registry.command(
        MAKE_MIGRATION_COMMAND,
        Command::new(MAKE_MIGRATION_COMMAND.as_str().to_string())
            .about("Generate a Rust migration scaffold")
            .arg(required_name_arg(
                "SLUG",
                "Migration slug to include in the timestamped filename",
            ))
            .arg(force_arg()),
        |invocation| async move { make_migration_command(invocation).await },
    )?;
    registry.command(
        MAKE_SEEDER_COMMAND,
        Command::new(MAKE_SEEDER_COMMAND.as_str().to_string())
            .about("Generate a Rust seeder scaffold")
            .arg(required_name_arg("NAME", "Seeder name to generate"))
            .arg(force_arg()),
        |invocation| async move { make_seeder_command(invocation).await },
    )?;
    registry.command(
        MAKE_MODEL_COMMAND,
        Command::new(MAKE_MODEL_COMMAND.as_str().to_string())
            .about("Generate a Rust model scaffold")
            .arg(required_name_arg(
                "NAME",
                "Model name in PascalCase (e.g. User, SendWelcomeEmail)",
            ))
            .arg(output_path_arg("src/app/models"))
            .arg(
                Arg::new("table")
                    .long("table")
                    .value_name("TABLE")
                    .help("Explicit PostgreSQL table name for the generated model"),
            )
            .arg(force_arg()),
        |invocation| async move { make_model_command(invocation).await },
    )?;
    registry.command(
        MAKE_JOB_COMMAND,
        Command::new(MAKE_JOB_COMMAND.as_str().to_string())
            .about("Generate a Rust job scaffold")
            .arg(required_name_arg(
                "NAME",
                "Job name in PascalCase (e.g. SendWelcomeEmail)",
            ))
            .arg(output_path_arg("src/app/jobs"))
            .arg(force_arg()),
        |invocation| async move { make_job_command(invocation).await },
    )?;
    registry.command(
        MAKE_COMMAND_COMMAND,
        Command::new(MAKE_COMMAND_COMMAND.as_str().to_string())
            .about("Generate a Rust CLI command scaffold")
            .arg(required_name_arg(
                "NAME",
                "Command name in PascalCase (e.g. SyncInventory)",
            ))
            .arg(output_path_arg("src/app/commands"))
            .arg(force_arg()),
        |invocation| async move { make_command_command(invocation).await },
    )?;
    for component in ComponentScaffold::ALL {
        register_component_command(registry, component)?;
    }
    Ok(())
}

fn register_component_command(
    registry: &mut CommandRegistry,
    component: ComponentScaffold,
) -> Result<()> {
    let id = component.command_id();
    let mut command = Command::new(id.as_str().to_string())
        .about(component.about())
        .arg(required_name_arg(
            "NAME",
            "Rust component name in PascalCase",
        ))
        .arg(output_path_arg(component.default_path()))
        .arg(force_arg());
    if component == ComponentScaffold::Listener {
        command = command.arg(
            Arg::new("event")
                .long("event")
                .value_name("EVENT")
                .required(true)
                .help("Event type handled by the generated listener"),
        );
    }
    if component == ComponentScaffold::Datatable {
        command = command.arg(
            Arg::new("model")
                .long("model")
                .value_name("MODEL")
                .required(true)
                .help("Model type queried by the generated datatable"),
        );
    }
    registry.command(id, command, move |invocation| async move {
        make_component_command(invocation, component).await
    })?;
    Ok(())
}

fn required_name_arg(value_name: &'static str, help: &'static str) -> Arg {
    Arg::new("name")
        .long("name")
        .value_name(value_name)
        .required(true)
        .help(help)
}

fn force_arg() -> Arg {
    Arg::new("force")
        .long("force")
        .action(ArgAction::SetTrue)
        .help("Overwrite an existing generated file")
}

fn output_path_arg(default_path: &'static str) -> Arg {
    Arg::new("path")
        .long("path")
        .value_name("DIR")
        .help(format!(
            "Directory to write the generated file into (default: {default_path})"
        ))
}

async fn make_migration_command(invocation: CommandInvocation) -> Result<()> {
    let config = invocation.app().config().database()?;
    let name = required_name(invocation.matches())?;
    let migration_dir = preferred_migrations_path(invocation.app(), &config)?;
    let basename = format!(
        "{}_{}",
        Local::now().format("%Y%m%d%H%M"),
        normalize_slug(name)
    );
    let filename = format!("{basename}.rs");
    let migration_path = write_scaffold_file(
        &migration_dir,
        &filename,
        render_migration_template(),
        invocation.matches().get_flag("force"),
    )?;

    invocation.line(format!("wrote {}", migration_path.display()))?;
    invocation.line("next: rebuild the app before running db:migrate so Foundry discovers it")?;
    Ok(())
}

async fn make_seeder_command(invocation: CommandInvocation) -> Result<()> {
    let config = invocation.app().config().database()?;
    let name = required_name(invocation.matches())?;
    let seeder_dir = preferred_seeders_path(invocation.app(), &config)?;
    let basename = to_snake_case(name);
    let filename = format!("{basename}.rs");
    let seeder_path = write_scaffold_file(
        &seeder_dir,
        &filename,
        render_seeder_template(),
        invocation.matches().get_flag("force"),
    )?;

    invocation.line(format!("wrote {}", seeder_path.display()))?;
    invocation.line("next: rebuild the app before running db:seed so Foundry discovers it")?;
    Ok(())
}

async fn make_model_command(invocation: CommandInvocation) -> Result<()> {
    let name = required_name(invocation.matches())?;
    let pascal = to_pascal_case(name);
    let snake = to_snake_case(name);
    let table = invocation
        .matches()
        .get_one::<String>("table")
        .map(|table| validate_table_name(table))
        .transpose()?
        .unwrap_or_else(|| pluralize_table_name(&snake));
    let model_dir = resolve_output_dir(invocation.matches(), "src/app/models")?;
    let filename = format!("{snake}.rs");
    let model_path = write_scaffold_file(
        &model_dir,
        &filename,
        render_model_template(&pascal, &table),
        invocation.matches().get_flag("force"),
    )?;

    invocation.line(format!("wrote {}", model_path.display()))?;
    invocation
        .line("next: add fields, create a migration, and register the module from your app")?;
    Ok(())
}

async fn make_job_command(invocation: CommandInvocation) -> Result<()> {
    let name = required_name(invocation.matches())?;
    let pascal = to_pascal_case(name);
    let snake = to_snake_case(name);
    let screaming = to_screaming_snake_case(&snake);
    let job_dir = resolve_output_dir(invocation.matches(), "src/app/jobs")?;
    let filename = format!("{snake}.rs");
    let job_path = write_scaffold_file(
        &job_dir,
        &filename,
        render_job_template(&pascal, &snake, &screaming),
        invocation.matches().get_flag("force"),
    )?;

    invocation.line(format!("wrote {}", job_path.display()))?;
    invocation.line("next: implement handle(), then register the job in a service provider")?;
    Ok(())
}

async fn make_command_command(invocation: CommandInvocation) -> Result<()> {
    let name = required_name(invocation.matches())?;
    let pascal = to_pascal_case(name);
    let snake = to_snake_case(name);
    let screaming = to_screaming_snake_case(&snake);
    let command_dir = resolve_output_dir(invocation.matches(), "src/app/commands")?;
    let filename = format!("{snake}.rs");
    let command_path = write_scaffold_file(
        &command_dir,
        &filename,
        render_command_template(&pascal, &snake, &screaming),
        invocation.matches().get_flag("force"),
    )?;

    invocation.line(format!("wrote {}", command_path.display()))?;
    invocation.line("next: call register() from your command registrar")?;
    Ok(())
}

async fn make_component_command(
    invocation: CommandInvocation,
    component: ComponentScaffold,
) -> Result<()> {
    let name = required_name(invocation.matches())?;
    let pascal = to_pascal_case(name);
    let snake = to_snake_case(name);
    let screaming = to_screaming_snake_case(&snake);
    let related_type = match component {
        ComponentScaffold::Listener => Some(required_related_type(invocation.matches(), "event")?),
        ComponentScaffold::Datatable => Some(required_related_type(invocation.matches(), "model")?),
        _ => None,
    };
    let output_dir = resolve_output_dir(invocation.matches(), component.default_path())?;
    let filename = format!("{snake}.rs");
    let contents = render_component_template(
        component,
        &pascal,
        &snake,
        &screaming,
        related_type.as_deref(),
    )?;
    let path = write_scaffold_file(
        &output_dir,
        &filename,
        contents,
        invocation.matches().get_flag("force"),
    )?;

    invocation.line(format!("wrote {}", path.display()))?;
    invocation.line(component.next_step())?;
    Ok(())
}

fn required_name(matches: &ArgMatches) -> Result<&str> {
    matches
        .get_one::<String>("name")
        .map(String::as_str)
        .ok_or_else(|| Error::message("missing required `--name` argument"))
}

fn required_related_type(matches: &ArgMatches, name: &str) -> Result<String> {
    let value = matches
        .get_one::<String>(name)
        .ok_or_else(|| Error::message(format!("missing required `--{name}` argument")))?;
    let pascal = to_pascal_case(value);
    if pascal.is_empty() {
        return Err(Error::message(format!("--{name} must name a Rust type")));
    }
    Ok(pascal)
}

fn preferred_migrations_path(app: &AppContext, config: &DatabaseConfig) -> Result<PathBuf> {
    if let Ok(paths) = app.resolve::<GeneratedDatabasePaths>() {
        if let Some(path) = paths.primary_migration_dir() {
            return Ok(path.to_path_buf());
        }
    }

    resolve_path(&config.migrations_path)
}

fn preferred_seeders_path(app: &AppContext, config: &DatabaseConfig) -> Result<PathBuf> {
    if let Ok(paths) = app.resolve::<GeneratedDatabasePaths>() {
        if let Some(path) = paths.primary_seeder_dir() {
            return Ok(path.to_path_buf());
        }
    }

    resolve_path(&config.seeders_path)
}

fn resolve_output_dir(matches: &ArgMatches, default_relative: &str) -> Result<PathBuf> {
    let path = matches
        .get_one::<String>("path")
        .map(String::as_str)
        .unwrap_or(default_relative);
    resolve_path(path)
}

fn resolve_path(path: &str) -> Result<PathBuf> {
    let configured = PathBuf::from(path);
    if configured.is_absolute() {
        return Ok(configured);
    }

    let cwd = std::env::current_dir().map_err(Error::other)?;
    Ok(cwd.join(configured))
}

fn write_scaffold_file(
    output_dir: &Path,
    filename: &str,
    contents: impl AsRef<[u8]>,
    force: bool,
) -> Result<PathBuf> {
    fs::create_dir_all(output_dir).map_err(Error::other)?;
    let relative = Path::new(filename);
    ensure_generated_file_writable(output_dir, relative, force)?;
    write_generated_file(output_dir, relative, contents)?;
    Ok(output_dir.join(relative))
}

fn render_migration_template() -> String {
    "use async_trait::async_trait;\nuse foundry::prelude::*;\n\npub struct Entry;\n\n#[async_trait]\nimpl MigrationFile for Entry {\n    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {\n        ctx.raw_execute(\n            r#\"CREATE TABLE your_table (id UUID PRIMARY KEY DEFAULT uuidv7());\"#,\n            &[],\n        )\n        .await?;\n        Ok(())\n    }\n\n    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {\n        ctx.raw_execute(\n            r#\"DROP TABLE IF EXISTS your_table;\"#,\n            &[],\n        )\n        .await?;\n        Ok(())\n    }\n}\n"
        .to_string()
}

fn render_seeder_template() -> String {
    "use async_trait::async_trait;\nuse foundry::prelude::*;\n\npub struct Entry;\n\n#[async_trait]\nimpl SeederFile for Entry {\n    async fn run(ctx: &SeederContext<'_>) -> Result<()> {\n        Query::insert_into(\"your_table\")\n            .value_expr(\"id\", Sql::uuid_v7())\n            .execute(ctx)\n            .await?;\n        Ok(())\n    }\n}\n"
        .to_string()
}

fn normalize_slug(input: &str) -> String {
    let slug = input
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();

    let slug = slug
        .split('_')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("_");

    if slug.is_empty() {
        "migration".to_string()
    } else {
        slug
    }
}

fn to_snake_case(value: &str) -> String {
    let mut output = String::new();
    let mut previous_was_separator = true;

    for character in value.chars() {
        if !character.is_ascii_alphanumeric() {
            if !output.ends_with('_') {
                output.push('_');
            }
            previous_was_separator = true;
            continue;
        }

        if character.is_ascii_uppercase() && !previous_was_separator && !output.ends_with('_') {
            output.push('_');
        }

        output.push(character.to_ascii_lowercase());
        previous_was_separator = false;
    }

    let collapsed = output
        .split('_')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("_");

    if collapsed.is_empty() {
        "generated_seeder".to_string()
    } else {
        collapsed
    }
}

fn to_pascal_case(value: &str) -> String {
    let snake = to_snake_case(value);
    snake
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            match chars.next() {
                Some(first) => {
                    let mut word = first.to_ascii_uppercase().to_string();
                    word.extend(chars);
                    word
                }
                None => String::new(),
            }
        })
        .collect()
}

fn to_screaming_snake_case(snake: &str) -> String {
    snake.to_ascii_uppercase()
}

fn validate_table_name(table: &str) -> Result<String> {
    let mut characters = table.chars();
    let valid_first = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_lowercase());
    let valid_rest = characters.all(|character| {
        character == '_' || character.is_ascii_lowercase() || character.is_ascii_digit()
    });
    if !valid_first || !valid_rest {
        return Err(Error::message(format!(
            "invalid model table name `{table}`; use a lowercase PostgreSQL identifier"
        )));
    }
    Ok(table.to_string())
}

fn pluralize_table_name(singular: &str) -> String {
    if singular.len() > 1
        && singular.ends_with('y')
        && singular
            .as_bytes()
            .get(singular.len() - 2)
            .is_some_and(|character| !matches!(character, b'a' | b'e' | b'i' | b'o' | b'u'))
    {
        return format!("{}ies", &singular[..singular.len() - 1]);
    }
    if singular.ends_with('s')
        || singular.ends_with('x')
        || singular.ends_with('z')
        || singular.ends_with("ch")
        || singular.ends_with("sh")
    {
        return format!("{singular}es");
    }
    format!("{singular}s")
}

fn render_model_template(pascal: &str, table_name: &str) -> String {
    format!(
        "use foundry::prelude::*;\n\
         \n\
         #[derive(Debug, foundry::Model)]\n\
         #[foundry(table = \"{table_name}\")]\n\
         pub struct {pascal} {{\n\
         \x20   pub id: ModelId<{pascal}>,\n\
         \x20   pub created_at: DateTime,\n\
         \x20   pub updated_at: DateTime,\n\
         }}\n"
    )
}

fn render_job_template(pascal: &str, snake: &str, screaming: &str) -> String {
    format!(
        "use async_trait::async_trait;\n\
         use foundry::prelude::*;\n\
         \n\
         pub const {screaming}_JOB: JobId = JobId::new(\"{snake}\");\n\
         \n\
         #[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]\n\
         pub struct {pascal};\n\
         \n\
         #[async_trait]\n\
         impl Job for {pascal} {{\n\
         \x20   const ID: JobId = {screaming}_JOB;\n\
         \n\
         \x20   async fn handle(&self, _context: JobContext) -> Result<()> {{\n\
         \x20       Ok(())\n\
         \x20   }}\n\
         }}\n"
    )
}

fn render_command_template(pascal: &str, snake: &str, screaming: &str) -> String {
    format!(
        "use foundry::prelude::*;\n\
         \n\
         pub const {screaming}_COMMAND: CommandId = CommandId::new(\"{snake}\");\n\
         \n\
         pub fn register(registry: &mut CommandRegistry) -> Result<()> {{\n\
         \x20   registry.command(\n\
         \x20       {screaming}_COMMAND,\n\
         \x20       clap::Command::new(\"{snake}\").about(\"{pascal} command\"),\n\
         \x20       |_invocation: CommandInvocation| async move {{ Ok(()) }},\n\
         \x20   )?;\n\
         \x20   Ok(())\n\
         }}\n"
    )
}

fn render_component_template(
    component: ComponentScaffold,
    pascal: &str,
    snake: &str,
    screaming: &str,
    related_type: Option<&str>,
) -> Result<String> {
    Ok(match component {
        ComponentScaffold::Request => render_request_template(pascal),
        ComponentScaffold::Dto => render_dto_template(pascal),
        ComponentScaffold::Policy => render_policy_template(pascal),
        ComponentScaffold::Event => render_event_template(pascal, snake, screaming),
        ComponentScaffold::Listener => render_listener_template(
            pascal,
            related_type.ok_or_else(|| Error::message("listener scaffold requires an event"))?,
        ),
        ComponentScaffold::Notification => render_notification_template(pascal, snake),
        ComponentScaffold::Mail => render_mail_template(pascal),
        ComponentScaffold::Datatable => render_datatable_template(
            pascal,
            snake,
            related_type.ok_or_else(|| Error::message("datatable scaffold requires a model"))?,
        ),
        ComponentScaffold::Plugin => render_plugin_template(pascal, snake),
        ComponentScaffold::Test => render_test_template(snake),
    })
}

fn render_request_template(pascal: &str) -> String {
    format!(
        "use foundry::prelude::*;\n\
         \n\
         #[derive(\n\
         \x20   Debug,\n\
         \x20   serde::Deserialize,\n\
         \x20   foundry::ts_rs::TS,\n\
         \x20   foundry::TS,\n\
         \x20   foundry::ApiSchema,\n\
         \x20   foundry::Validate,\n\
         )]\n\
         pub struct {pascal} {{\n\
         \x20   #[validate(required)]\n\
         \x20   pub name: String,\n\
         }}\n"
    )
}

fn render_dto_template(pascal: &str) -> String {
    format!(
        "use foundry::prelude::*;\n\
         \n\
         #[derive(\n\
         \x20   Clone,\n\
         \x20   Debug,\n\
         \x20   serde::Serialize,\n\
         \x20   serde::Deserialize,\n\
         \x20   foundry::ts_rs::TS,\n\
         \x20   foundry::TS,\n\
         \x20   foundry::ApiSchema,\n\
         )]\n\
         pub struct {pascal} {{\n\
         \x20   pub id: String,\n\
         }}\n"
    )
}

fn render_policy_template(pascal: &str) -> String {
    format!(
        "use async_trait::async_trait;\n\
         use foundry::prelude::*;\n\
         \n\
         pub struct {pascal};\n\
         \n\
         #[async_trait]\n\
         impl Policy for {pascal} {{\n\
         \x20   async fn evaluate(&self, _actor: &Actor, _app: &AppContext) -> Result<bool> {{\n\
         \x20       Ok(false)\n\
         \x20   }}\n\
         }}\n"
    )
}

fn render_event_template(pascal: &str, snake: &str, screaming: &str) -> String {
    format!(
        "use foundry::prelude::*;\n\
         \n\
         pub const {screaming}_EVENT: EventId = EventId::new(\"{snake}\");\n\
         \n\
         #[derive(Clone, Debug, serde::Serialize)]\n\
         pub struct {pascal};\n\
         \n\
         impl Event for {pascal} {{\n\
         \x20   const ID: EventId = {screaming}_EVENT;\n\
         }}\n"
    )
}

fn render_listener_template(pascal: &str, event: &str) -> String {
    let event_snake = to_snake_case(event);
    format!(
        "use async_trait::async_trait;\n\
         use foundry::prelude::*;\n\
         \n\
         use crate::app::events::{event_snake}::{event};\n\
         \n\
         pub struct {pascal};\n\
         \n\
         #[async_trait]\n\
         impl EventListener<{event}> for {pascal} {{\n\
         \x20   async fn handle(&self, _context: &EventContext, _event: &{event}) -> Result<()> {{\n\
         \x20       Ok(())\n\
         \x20   }}\n\
         }}\n"
    )
}

fn render_notification_template(pascal: &str, snake: &str) -> String {
    format!(
        "use foundry::prelude::*;\n\
         \n\
         pub struct {pascal};\n\
         \n\
         impl Notification for {pascal} {{\n\
         \x20   fn notification_type(&self) -> &str {{\n\
         \x20       \"{snake}\"\n\
         \x20   }}\n\
         \n\
         \x20   fn via(&self) -> Vec<NotificationChannelId> {{\n\
         \x20       vec![NOTIFY_DATABASE]\n\
         \x20   }}\n\
         \n\
         \x20   fn to_database(&self) -> Option<serde_json::Value> {{\n\
         \x20       Some(serde_json::json!({{}}))\n\
         \x20   }}\n\
         }}\n"
    )
}

fn render_mail_template(pascal: &str) -> String {
    format!(
        "use foundry::prelude::*;\n\
         \n\
         pub struct {pascal};\n\
         \n\
         impl {pascal} {{\n\
         \x20   pub fn build(recipient: impl Into<EmailAddress>) -> EmailMessage {{\n\
         \x20       EmailMessage::new(\"{pascal}\")\n\
         \x20           .to(recipient)\n\
         \x20           .text_body(\"{pascal}\")\n\
         \x20   }}\n\
         }}\n"
    )
}

fn render_datatable_template(pascal: &str, snake: &str, model: &str) -> String {
    let model_snake = to_snake_case(model);
    format!(
        "use async_trait::async_trait;\n\
         use foundry::prelude::*;\n\
         \n\
         use crate::app::models::{model_snake}::{model};\n\
         \n\
         pub struct {pascal};\n\
         \n\
         #[async_trait]\n\
         impl Datatable for {pascal} {{\n\
         \x20   type Row = {model};\n\
         \x20   type Query = ModelQuery<{model}>;\n\
         \n\
         \x20   const ID: &'static str = \"{snake}\";\n\
         \n\
         \x20   fn query(_context: &DatatableContext) -> Self::Query {{\n\
         \x20       {model}::query()\n\
         \x20   }}\n\
         \n\
         \x20   fn columns() -> Vec<DatatableColumn<Self::Row>> {{\n\
         \x20       vec![\n\
         \x20           DatatableColumn::field({model}::ID)\n\
         \x20               .label(\"ID\")\n\
         \x20               .sortable()\n\
         \x20               .exportable(),\n\
         \x20       ]\n\
         \x20   }}\n\
         }}\n"
    )
}

fn render_plugin_template(pascal: &str, snake: &str) -> String {
    format!(
        "use foundry::prelude::*;\n\
         \n\
         pub struct {pascal};\n\
         \n\
         impl Plugin for {pascal} {{\n\
         \x20   fn manifest(&self) -> PluginManifest {{\n\
         \x20       PluginManifest::new(\n\
         \x20           \"{snake}\",\n\
         \x20           \"0.1.0\".parse().expect(\"valid plugin version\"),\n\
         \x20           \">=0.1\".parse().expect(\"valid Foundry version requirement\"),\n\
         \x20       )\n\
         \x20   }}\n\
         \n\
         \x20   fn register(&self, _registrar: &mut PluginRegistrar) -> Result<()> {{\n\
         \x20       Ok(())\n\
         \x20   }}\n\
         }}\n"
    )
}

fn render_test_template(snake: &str) -> String {
    format!(
        "use foundry::prelude::*;\n\
         \n\
         #[tokio::test]\n\
         async fn {snake}() -> Result<()> {{\n\
         \x20   let app = TestApp::builder().build().await?;\n\
         \x20   app.shutdown().await\n\
         }}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::{
        pluralize_table_name, render_command_template, render_component_template,
        render_job_template, render_model_template, to_pascal_case, to_screaming_snake_case,
        validate_table_name, write_scaffold_file, ComponentScaffold,
    };

    #[test]
    fn to_pascal_case_from_snake() {
        assert_eq!(to_pascal_case("send_welcome_email"), "SendWelcomeEmail");
    }

    #[test]
    fn to_pascal_case_from_pascal() {
        assert_eq!(to_pascal_case("SendWelcomeEmail"), "SendWelcomeEmail");
    }

    #[test]
    fn to_pascal_case_single_word() {
        assert_eq!(to_pascal_case("user"), "User");
        assert_eq!(to_pascal_case("User"), "User");
    }

    #[test]
    fn model_table_pluralization_is_conservative_and_override_is_validated() {
        assert_eq!(pluralize_table_name("category"), "categories");
        assert_eq!(pluralize_table_name("status"), "statuses");
        assert_eq!(pluralize_table_name("box"), "boxes");
        assert_eq!(pluralize_table_name("key"), "keys");
        assert_eq!(pluralize_table_name("person"), "persons");

        assert_eq!(
            validate_table_name("audit_entries").unwrap(),
            "audit_entries"
        );
        assert!(validate_table_name("AuditEntries").is_err());
        assert!(validate_table_name("audit.entries").is_err());
    }

    #[test]
    fn to_screaming_snake_case_converts() {
        assert_eq!(
            to_screaming_snake_case("send_welcome_email"),
            "SEND_WELCOME_EMAIL"
        );
    }

    #[test]
    fn render_model_template_contains_struct() {
        let output = render_model_template("User", "users");
        assert!(output.contains("pub struct User {"));
        assert!(output.contains("#[foundry(table = \"users\")]"));
        assert!(output.contains("pub id: ModelId<User>"));
        assert!(output.contains("#[derive(Debug, foundry::Model)]"));
        assert!(!output.contains("#[derive(Clone"));
    }

    #[test]
    fn render_job_template_contains_const_and_impl_without_placeholder_todo() {
        let output = render_job_template(
            "SendWelcomeEmail",
            "send_welcome_email",
            "SEND_WELCOME_EMAIL",
        );
        assert!(output.contains("pub const SEND_WELCOME_EMAIL_JOB: JobId"));
        assert!(output.contains("pub struct SendWelcomeEmail;"));
        assert!(output.contains("const ID: JobId = SEND_WELCOME_EMAIL_JOB;"));
        assert!(!output.contains("TODO"));
    }

    #[test]
    fn render_command_template_contains_register_without_placeholder_todo() {
        let output = render_command_template("SyncInventory", "sync_inventory", "SYNC_INVENTORY");
        assert!(output.contains("pub const SYNC_INVENTORY_COMMAND: CommandId"));
        assert!(output.contains("pub fn register("));
        assert!(output.contains("Command::new(\"sync_inventory\")"));
        assert!(!output.contains("TODO"));
    }

    #[test]
    fn component_templates_cover_each_repeated_application_artifact() {
        for component in ComponentScaffold::ALL {
            let related = match component {
                ComponentScaffold::Listener => Some("UserCreated"),
                ComponentScaffold::Datatable => Some("User"),
                _ => None,
            };
            let output = render_component_template(
                component,
                "GeneratedComponent",
                "generated_component",
                "GENERATED_COMPONENT",
                related,
            )
            .unwrap();

            assert!(output.ends_with('\n'), "{component:?}");
            assert!(!output.contains("TODO"), "{component:?}");
            assert!(
                output.contains("GeneratedComponent") || component == ComponentScaffold::Test,
                "{component:?}"
            );
        }

        let request = render_component_template(
            ComponentScaffold::Request,
            "CreateUserRequest",
            "create_user_request",
            "CREATE_USER_REQUEST",
            None,
        )
        .unwrap();
        assert!(request.contains("foundry::Validate"));
        assert!(request.contains("foundry::ApiSchema"));

        let listener = render_component_template(
            ComponentScaffold::Listener,
            "SendWelcomeEmail",
            "send_welcome_email",
            "SEND_WELCOME_EMAIL",
            Some("UserCreated"),
        )
        .unwrap();
        assert!(listener.contains("EventListener<UserCreated>"));
        assert!(listener.contains("events::user_created::UserCreated"));

        let datatable = render_component_template(
            ComponentScaffold::Datatable,
            "UsersDatatable",
            "users_datatable",
            "USERS_DATATABLE",
            Some("User"),
        )
        .unwrap();
        assert!(datatable.contains("type Query = ModelQuery<User>"));

        let plugin = render_component_template(
            ComponentScaffold::Plugin,
            "AuditPlugin",
            "audit_plugin",
            "AUDIT_PLUGIN",
            None,
        )
        .unwrap();
        assert!(plugin.contains("impl Plugin for AuditPlugin"));
    }

    #[cfg(unix)]
    #[test]
    fn scaffold_writer_allows_symlinked_output_root() {
        use std::fs;
        use std::os::unix::fs::symlink;

        let holder = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let output_root = holder.path().join("models");
        symlink(target.path(), &output_root).unwrap();

        write_scaffold_file(&output_root, "user.rs", "pub struct User;\n", false).unwrap();

        assert_eq!(
            fs::read_to_string(target.path().join("user.rs")).unwrap(),
            "pub struct User;\n"
        );
    }
}
