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
const MAKE_IDS_COMMAND: CommandId = CommandId::new("make:ids");
const MAKE_MODEL_COMMAND: CommandId = CommandId::new("make:model");
const MAKE_REQUEST_COMMAND: CommandId = CommandId::new("make:request");
const MAKE_RESPONSE_COMMAND: CommandId = CommandId::new("make:response");
const MAKE_GUARD_COMMAND: CommandId = CommandId::new("make:guard");
const MAKE_POLICY_COMMAND: CommandId = CommandId::new("make:policy");
const MAKE_JOB_COMMAND: CommandId = CommandId::new("make:job");
const MAKE_EVENT_COMMAND: CommandId = CommandId::new("make:event");
const MAKE_NOTIFICATION_COMMAND: CommandId = CommandId::new("make:notification");
const MAKE_COMMAND_COMMAND: CommandId = CommandId::new("make:command");

pub(crate) fn scaffold_cli_registrar() -> CommandRegistrar {
    Arc::new(register_cli_commands)
}

fn register_cli_commands(registry: &mut CommandRegistry) -> Result<()> {
    registry.command(
        MAKE_MIGRATION_COMMAND,
        Command::new(MAKE_MIGRATION_COMMAND.as_str().to_string())
            .about("Generate a raw SQL Rust migration scaffold")
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
        MAKE_IDS_COMMAND,
        Command::new(MAKE_IDS_COMMAND.as_str().to_string())
            .about("Generate a typed app IDs scaffold")
            .arg(output_path_arg("src/app"))
            .arg(force_arg()),
        |invocation| async move { make_ids_command(invocation).await },
    )?;
    registry.command(
        MAKE_MODEL_COMMAND,
        Command::new(MAKE_MODEL_COMMAND.as_str().to_string())
            .about("Generate a typed Rust model scaffold")
            .arg(required_name_arg(
                "NAME",
                "Model name in PascalCase (e.g. User, SendWelcomeEmail)",
            ))
            .arg(output_path_arg("src/app/models"))
            .arg(force_arg()),
        |invocation| async move { make_model_command(invocation).await },
    )?;
    registry.command(
        MAKE_REQUEST_COMMAND,
        Command::new(MAKE_REQUEST_COMMAND.as_str().to_string())
            .about("Generate a typed Rust request DTO scaffold")
            .arg(required_name_arg(
                "NAME",
                "Request DTO name in PascalCase (e.g. StorePostRequest)",
            ))
            .arg(output_path_arg("src/app/requests"))
            .arg(force_arg()),
        |invocation| async move { make_request_command(invocation).await },
    )?;
    registry.command(
        MAKE_RESPONSE_COMMAND,
        Command::new(MAKE_RESPONSE_COMMAND.as_str().to_string())
            .about("Generate a typed Rust response DTO scaffold")
            .arg(required_name_arg(
                "NAME",
                "Response DTO name in PascalCase (e.g. PostResponse)",
            ))
            .arg(output_path_arg("src/app/responses"))
            .arg(force_arg()),
        |invocation| async move { make_response_command(invocation).await },
    )?;
    registry.command(
        MAKE_GUARD_COMMAND,
        Command::new(MAKE_GUARD_COMMAND.as_str().to_string())
            .about("Generate a Rust auth guard scaffold")
            .arg(required_name_arg(
                "NAME",
                "Guard authenticator name in PascalCase (e.g. ApiGuard)",
            ))
            .arg(output_path_arg("src/app/guards"))
            .arg(force_arg()),
        |invocation| async move { make_guard_command(invocation).await },
    )?;
    registry.command(
        MAKE_POLICY_COMMAND,
        Command::new(MAKE_POLICY_COMMAND.as_str().to_string())
            .about("Generate a Rust auth policy scaffold")
            .arg(required_name_arg(
                "NAME",
                "Policy name in PascalCase (e.g. CanEditPost)",
            ))
            .arg(output_path_arg("src/app/policies"))
            .arg(force_arg()),
        |invocation| async move { make_policy_command(invocation).await },
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
        MAKE_EVENT_COMMAND,
        Command::new(MAKE_EVENT_COMMAND.as_str().to_string())
            .about("Generate a typed Rust event scaffold")
            .arg(required_name_arg(
                "NAME",
                "Event name in PascalCase (e.g. OrderPlaced)",
            ))
            .arg(output_path_arg("src/app/events"))
            .arg(force_arg()),
        |invocation| async move { make_event_command(invocation).await },
    )?;
    registry.command(
        MAKE_NOTIFICATION_COMMAND,
        Command::new(MAKE_NOTIFICATION_COMMAND.as_str().to_string())
            .about("Generate a typed Rust notification scaffold")
            .arg(required_name_arg(
                "NAME",
                "Notification name in PascalCase (e.g. OrderShipped)",
            ))
            .arg(output_path_arg("src/app/notifications"))
            .arg(force_arg()),
        |invocation| async move { make_notification_command(invocation).await },
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
    let migration_path = migration_dir.join(format!("{basename}.rs"));

    fs::create_dir_all(&migration_dir).map_err(Error::other)?;
    ensure_writable(&migration_path, invocation.matches().get_flag("force"))?;
    write_generated_file(&migration_path, render_migration_template())?;

    println!("wrote {}", migration_path.display());
    println!("next: rebuild the app before running db:migrate so Foundry discovers it");
    Ok(())
}

async fn make_seeder_command(invocation: CommandInvocation) -> Result<()> {
    let config = invocation.app().config().database()?;
    let name = required_name(invocation.matches())?;
    let seeder_dir = preferred_seeders_path(invocation.app(), &config)?;
    let basename = to_snake_case(name);
    let seeder_path = seeder_dir.join(format!("{basename}.rs"));

    fs::create_dir_all(&seeder_dir).map_err(Error::other)?;
    ensure_writable(&seeder_path, invocation.matches().get_flag("force"))?;
    write_generated_file(&seeder_path, render_seeder_template())?;

    println!("wrote {}", seeder_path.display());
    println!("next: rebuild the app before running db:seed so Foundry discovers it");
    Ok(())
}

async fn make_ids_command(invocation: CommandInvocation) -> Result<()> {
    let ids_dir = resolve_output_dir(invocation.matches(), "src/app")?;
    let ids_path = ids_dir.join("ids.rs");

    fs::create_dir_all(&ids_dir).map_err(Error::other)?;
    ensure_writable(&ids_path, invocation.matches().get_flag("force"))?;
    write_generated_file(&ids_path, render_ids_template())?;

    println!("wrote {}", ids_path.display());
    println!("next: add variants as the app gains guards, permissions, and named routes");
    Ok(())
}

async fn make_model_command(invocation: CommandInvocation) -> Result<()> {
    let name = required_name(invocation.matches())?;
    let pascal = to_pascal_case(name);
    let snake = to_snake_case(name);
    let model_dir = resolve_output_dir(invocation.matches(), "src/app/models")?;
    let model_path = model_dir.join(format!("{snake}.rs"));

    fs::create_dir_all(&model_dir).map_err(Error::other)?;
    ensure_writable(&model_path, invocation.matches().get_flag("force"))?;
    write_generated_file(&model_path, render_model_template(&pascal, &snake))?;

    println!("wrote {}", model_path.display());
    println!("next: add fields, create a migration, and register the module from your app");
    Ok(())
}

async fn make_request_command(invocation: CommandInvocation) -> Result<()> {
    let name = required_name(invocation.matches())?;
    let pascal = to_pascal_case(name);
    let snake = to_snake_case(name);
    let request_dir = resolve_output_dir(invocation.matches(), "src/app/requests")?;
    let request_path = request_dir.join(format!("{snake}.rs"));

    fs::create_dir_all(&request_dir).map_err(Error::other)?;
    ensure_writable(&request_path, invocation.matches().get_flag("force"))?;
    write_generated_file(&request_path, render_request_template(&pascal))?;

    println!("wrote {}", request_path.display());
    println!("next: add fields, then use JsonValidated<T> or Validated<T> in a route");
    Ok(())
}

async fn make_response_command(invocation: CommandInvocation) -> Result<()> {
    let name = required_name(invocation.matches())?;
    let pascal = to_pascal_case(name);
    let snake = to_snake_case(name);
    let response_dir = resolve_output_dir(invocation.matches(), "src/app/responses")?;
    let response_path = response_dir.join(format!("{snake}.rs"));

    fs::create_dir_all(&response_dir).map_err(Error::other)?;
    ensure_writable(&response_path, invocation.matches().get_flag("force"))?;
    write_generated_file(&response_path, render_response_template(&pascal))?;

    println!("wrote {}", response_path.display());
    println!("next: add fields, then document routes with route.response::<T>(status)");
    Ok(())
}

async fn make_guard_command(invocation: CommandInvocation) -> Result<()> {
    let name = required_name(invocation.matches())?;
    let pascal = to_pascal_case(name);
    let snake = to_snake_case(name);
    let guard_id = trim_snake_suffix(&snake, "_guard");
    let screaming = to_screaming_snake_case(&guard_id);
    let guard_dir = resolve_output_dir(invocation.matches(), "src/app/guards")?;
    let guard_path = guard_dir.join(format!("{snake}.rs"));

    fs::create_dir_all(&guard_dir).map_err(Error::other)?;
    ensure_writable(&guard_path, invocation.matches().get_flag("force"))?;
    write_generated_file(
        &guard_path,
        render_guard_template(&pascal, &guard_id, &screaming),
    )?;

    println!("wrote {}", guard_path.display());
    println!(
        "next: implement authenticate(), configure auth.guards.{guard_id}, then call register()"
    );
    Ok(())
}

async fn make_policy_command(invocation: CommandInvocation) -> Result<()> {
    let name = required_name(invocation.matches())?;
    let pascal = to_pascal_case(name);
    let snake = to_snake_case(name);
    let screaming = to_screaming_snake_case(&snake);
    let policy_dir = resolve_output_dir(invocation.matches(), "src/app/policies")?;
    let policy_path = policy_dir.join(format!("{snake}.rs"));

    fs::create_dir_all(&policy_dir).map_err(Error::other)?;
    ensure_writable(&policy_path, invocation.matches().get_flag("force"))?;
    write_generated_file(
        &policy_path,
        render_policy_template(&pascal, &snake, &screaming),
    )?;

    println!("wrote {}", policy_path.display());
    println!("next: implement evaluate(), then call register() from your service provider");
    Ok(())
}

async fn make_job_command(invocation: CommandInvocation) -> Result<()> {
    let name = required_name(invocation.matches())?;
    let pascal = to_pascal_case(name);
    let snake = to_snake_case(name);
    let screaming = to_screaming_snake_case(&snake);
    let job_dir = resolve_output_dir(invocation.matches(), "src/app/jobs")?;
    let job_path = job_dir.join(format!("{snake}.rs"));

    fs::create_dir_all(&job_dir).map_err(Error::other)?;
    ensure_writable(&job_path, invocation.matches().get_flag("force"))?;
    write_generated_file(&job_path, render_job_template(&pascal, &snake, &screaming))?;

    println!("wrote {}", job_path.display());
    println!("next: implement handle(), then register the job in a service provider");
    Ok(())
}

async fn make_event_command(invocation: CommandInvocation) -> Result<()> {
    let name = required_name(invocation.matches())?;
    let pascal = to_pascal_case(name);
    let snake = to_snake_case(name);
    let screaming = to_screaming_snake_case(&snake);
    let event_id = to_event_id(&snake);
    let event_dir = resolve_output_dir(invocation.matches(), "src/app/events")?;
    let event_path = event_dir.join(format!("{snake}.rs"));

    fs::create_dir_all(&event_dir).map_err(Error::other)?;
    ensure_writable(&event_path, invocation.matches().get_flag("force"))?;
    write_generated_file(
        &event_path,
        render_event_template(&pascal, &event_id, &screaming),
    )?;

    println!("wrote {}", event_path.display());
    println!("next: add payload fields, then dispatch the event or register listeners");
    Ok(())
}

async fn make_notification_command(invocation: CommandInvocation) -> Result<()> {
    let name = required_name(invocation.matches())?;
    let pascal = to_pascal_case(name);
    let snake = to_snake_case(name);
    let screaming = to_screaming_snake_case(&snake);
    let notification_type = to_event_id(&snake);
    let notification_dir = resolve_output_dir(invocation.matches(), "src/app/notifications")?;
    let notification_path = notification_dir.join(format!("{snake}.rs"));

    fs::create_dir_all(&notification_dir).map_err(Error::other)?;
    ensure_writable(&notification_path, invocation.matches().get_flag("force"))?;
    write_generated_file(
        &notification_path,
        render_notification_template(&pascal, &notification_type, &screaming),
    )?;

    println!("wrote {}", notification_path.display());
    println!("next: add payload fields, then call app.notify() or app.notify_queued()");
    Ok(())
}

async fn make_command_command(invocation: CommandInvocation) -> Result<()> {
    let name = required_name(invocation.matches())?;
    let pascal = to_pascal_case(name);
    let snake = to_snake_case(name);
    let screaming = to_screaming_snake_case(&snake);
    let command_dir = resolve_output_dir(invocation.matches(), "src/app/commands")?;
    let command_path = command_dir.join(format!("{snake}.rs"));

    fs::create_dir_all(&command_dir).map_err(Error::other)?;
    ensure_writable(&command_path, invocation.matches().get_flag("force"))?;
    write_generated_file(
        &command_path,
        render_command_template(&pascal, &snake, &screaming),
    )?;

    println!("wrote {}", command_path.display());
    println!("next: call register() from your command registrar");
    Ok(())
}

fn required_name(matches: &ArgMatches) -> Result<&String> {
    matches
        .get_one::<String>("name")
        .ok_or_else(|| Error::message("missing required `--name` argument"))
}

fn preferred_migrations_path(app: &AppContext, config: &DatabaseConfig) -> Result<PathBuf> {
    if let Ok(paths) = app.resolve::<GeneratedDatabasePaths>() {
        if let Some(path) = paths.primary_migration_dir() {
            return Ok(path.to_path_buf());
        }
    }

    resolve_configured_path(&config.migrations_path)
}

fn preferred_seeders_path(app: &AppContext, config: &DatabaseConfig) -> Result<PathBuf> {
    if let Ok(paths) = app.resolve::<GeneratedDatabasePaths>() {
        if let Some(path) = paths.primary_seeder_dir() {
            return Ok(path.to_path_buf());
        }
    }

    resolve_configured_path(&config.seeders_path)
}

fn resolve_configured_path(path: &str) -> Result<PathBuf> {
    resolve_path(path)
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

fn ensure_writable(path: &Path, force: bool) -> Result<()> {
    ensure_generated_file_writable(path, force)
}

fn render_migration_template() -> String {
    "use foundry::prelude::*;\n\npub struct Entry;\n\n#[foundry::async_trait]\nimpl MigrationFile for Entry {\n    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {\n        ctx.raw_execute(\n            r#\"CREATE TABLE your_table (id UUID PRIMARY KEY DEFAULT uuidv7());\"#,\n            &[],\n        )\n        .await?;\n        Ok(())\n    }\n\n    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {\n        ctx.raw_execute(\n            r#\"DROP TABLE IF EXISTS your_table;\"#,\n            &[],\n        )\n        .await?;\n        Ok(())\n    }\n}\n"
        .to_string()
}

fn render_seeder_template() -> String {
    "use foundry::prelude::*;\n\npub struct Entry;\n\n#[foundry::async_trait]\nimpl SeederFile for Entry {\n    async fn run(ctx: &SeederContext<'_>) -> Result<()> {\n        Query::insert_into(\"your_table\")\n            .value_expr(\"id\", Sql::uuid_v7())\n            .execute(ctx)\n            .await?;\n        Ok(())\n    }\n}\n"
        .to_string()
}

fn render_ids_template() -> String {
    "use foundry::prelude::*;\n\n#[derive(Clone, Copy, FoundryId)]\n#[foundry(id = GuardId, rename_all = \"snake_case\")]\npub enum AuthGuard {\n    Api,\n}\n\n#[derive(Clone, Copy, Debug, PartialEq, Eq, AppEnum)]\n#[foundry(id = \"ability\", id_type = PermissionId)]\npub enum Ability {\n    #[foundry(key = \"dashboard:view\")]\n    DashboardView,\n}\n\n#[derive(Clone, Copy, FoundryId)]\n#[foundry(id = RouteId)]\npub enum Route {\n    #[foundry(value = \"health\")]\n    Health,\n}\n"
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

fn trim_snake_suffix(value: &str, suffix: &str) -> String {
    value
        .strip_suffix(suffix)
        .filter(|trimmed| !trimmed.is_empty())
        .unwrap_or(value)
        .to_string()
}

fn to_event_id(snake: &str) -> String {
    snake.replace('_', ".")
}

fn render_model_template(pascal: &str, snake: &str) -> String {
    let table_name = format!("{snake}s");
    format!(
        "use foundry::prelude::*;\n\
         \n\
         #[derive(\n\
         \x20   Debug,\n\
         \x20   serde::Serialize,\n\
         \x20   foundry::ts_rs::TS,\n\
         \x20   foundry::ApiSchema,\n\
         \x20   foundry::Model,\n\
         )]\n\
         #[ts(crate = \"foundry::ts_rs\")]\n\
         #[foundry(table = \"{table_name}\")]\n\
         pub struct {pascal} {{\n\
         \x20   pub id: ModelId<{pascal}>,\n\
         \x20   pub created_at: DateTime,\n\
         \x20   pub updated_at: DateTime,\n\
         }}\n"
    )
}

fn render_request_template(pascal: &str) -> String {
    format!(
        "#[derive(\n\
         \x20   Debug,\n\
         \x20   serde::Deserialize,\n\
         \x20   foundry::ts_rs::TS,\n\
         \x20   foundry::ApiSchema,\n\
         \x20   foundry::Validate,\n\
         )]\n\
         #[ts(crate = \"foundry::ts_rs\")]\n\
         pub struct {pascal} {{\n\
         \x20   #[validate(required)]\n\
         \x20   pub name: String,\n\
         }}\n"
    )
}

fn render_response_template(pascal: &str) -> String {
    format!(
        "#[derive(\n\
         \x20   Debug,\n\
         \x20   serde::Serialize,\n\
         \x20   foundry::ts_rs::TS,\n\
         \x20   foundry::ApiSchema,\n\
         )]\n\
         #[ts(crate = \"foundry::ts_rs\")]\n\
         pub struct {pascal} {{\n\
         \x20   pub message: String,\n\
         }}\n"
    )
}

fn render_guard_template(pascal: &str, guard_id: &str, screaming: &str) -> String {
    format!(
        "use foundry::prelude::*;\n\
         \n\
         pub const {screaming}_GUARD: GuardId = GuardId::new(\"{guard_id}\");\n\
         \n\
         pub struct {pascal};\n\
         \n\
         pub fn register(registrar: &mut ServiceRegistrar) -> Result<()> {{\n\
         \x20   registrar.register_guard({screaming}_GUARD, {pascal})\n\
         }}\n\
         \n\
         #[foundry::async_trait]\n\
         impl BearerAuthenticator for {pascal} {{\n\
         \x20   async fn authenticate(&self, _token: &str) -> Result<Option<Actor>> {{\n\
         \x20       Ok(None)\n\
         \x20   }}\n\
         }}\n"
    )
}

fn render_policy_template(pascal: &str, snake: &str, screaming: &str) -> String {
    format!(
        "use foundry::prelude::*;\n\
         \n\
         pub const {screaming}_POLICY: PolicyId = PolicyId::new(\"{snake}\");\n\
         \n\
         pub struct {pascal};\n\
         \n\
         pub fn register(registrar: &mut ServiceRegistrar) -> Result<()> {{\n\
         \x20   registrar.register_policy({screaming}_POLICY, {pascal})\n\
         }}\n\
         \n\
         #[foundry::async_trait]\n\
         impl Policy for {pascal} {{\n\
         \x20   async fn evaluate(&self, _actor: &Actor, _app: &AppContext) -> Result<bool> {{\n\
         \x20       Ok(false)\n\
         \x20   }}\n\
         }}\n"
    )
}

fn render_event_template(pascal: &str, event_id: &str, screaming: &str) -> String {
    format!(
        "use foundry::prelude::*;\n\
         \n\
         pub const {screaming}_EVENT: EventId = EventId::new(\"{event_id}\");\n\
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
         #[ts(crate = \"foundry::ts_rs\")]\n\
         pub struct {pascal} {{}}\n\
         \n\
         impl Event for {pascal} {{\n\
         \x20   const ID: EventId = {screaming}_EVENT;\n\
         }}\n\
         \n\
         foundry::inventory::submit! {{\n\
         \x20   TsEventPayload::new({screaming}_EVENT, \"{pascal}\")\n\
         }}\n"
    )
}

fn render_notification_template(pascal: &str, notification_type: &str, screaming: &str) -> String {
    format!(
        "use foundry::prelude::*;\n\
         \n\
         pub const {screaming}_NOTIFICATION_TYPE: &str = \"{notification_type}\";\n\
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
         #[ts(crate = \"foundry::ts_rs\")]\n\
         pub struct {pascal} {{}}\n\
         \n\
         impl Notification for {pascal} {{\n\
         \x20   fn notification_type(&self) -> &str {{\n\
         \x20       {screaming}_NOTIFICATION_TYPE\n\
         \x20   }}\n\
         \n\
         \x20   fn via(&self) -> Vec<NotificationChannelId> {{\n\
         \x20       vec![NOTIFY_DATABASE, NOTIFY_BROADCAST]\n\
         \x20   }}\n\
         \n\
         \x20   fn to_database(&self) -> Option<foundry::serde_json::Value> {{\n\
         \x20       foundry::serde_json::to_value(self).ok()\n\
         \x20   }}\n\
         \n\
         \x20   fn to_broadcast(&self) -> Option<foundry::serde_json::Value> {{\n\
         \x20       foundry::serde_json::to_value(self).ok()\n\
         \x20   }}\n\
         }}\n\
         \n\
         foundry::inventory::submit! {{\n\
         \x20   TsNotification {{\n\
         \x20       notification_type: {screaming}_NOTIFICATION_TYPE,\n\
         \x20       payload: \"{pascal}\",\n\
         \x20   }}\n\
         }}\n"
    )
}

fn render_job_template(pascal: &str, snake: &str, screaming: &str) -> String {
    format!(
        "use foundry::prelude::*;\n\
         \n\
         pub const {screaming}_JOB: JobId = JobId::new(\"{snake}\");\n\
         \n\
         #[derive(\n\
         \x20   Clone,\n\
         \x20   Debug,\n\
         \x20   serde::Serialize,\n\
         \x20   serde::Deserialize,\n\
         \x20   foundry::ts_rs::TS,\n\
         \x20   foundry::TS,\n\
         )]\n\
         #[ts(crate = \"foundry::ts_rs\")]\n\
         pub struct {pascal};\n\
         \n\
         #[foundry::async_trait]\n\
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
         \x20       Command::new(\"{snake}\").about(\"{pascal} command\"),\n\
         \x20       |_invocation: CommandInvocation| async move {{ Ok(()) }},\n\
         \x20   )?;\n\
         \x20   Ok(())\n\
         }}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::{
        render_command_template, render_event_template, render_guard_template, render_ids_template,
        render_job_template, render_migration_template, render_model_template,
        render_notification_template, render_policy_template, render_request_template,
        render_response_template, render_seeder_template, to_event_id, to_pascal_case,
        to_screaming_snake_case, trim_snake_suffix,
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
    fn to_screaming_snake_case_converts() {
        assert_eq!(
            to_screaming_snake_case("send_welcome_email"),
            "SEND_WELCOME_EMAIL"
        );
    }

    #[test]
    fn trim_snake_suffix_removes_guard_suffix_without_emptying_value() {
        assert_eq!(trim_snake_suffix("api_guard", "_guard"), "api");
        assert_eq!(trim_snake_suffix("guard", "_guard"), "guard");
        assert_eq!(trim_snake_suffix("admin", "_guard"), "admin");
    }

    #[test]
    fn to_event_id_converts_snake_to_dot_notation() {
        assert_eq!(to_event_id("order_placed"), "order.placed");
    }

    #[test]
    fn render_model_template_contains_struct() {
        let output = render_model_template("User", "user");
        assert!(output.contains("pub struct User {"));
        assert!(output.contains("#[foundry(table = \"users\")]"));
        assert!(output.contains("pub id: ModelId<User>"));
        assert!(output.contains("#[ts(crate = \"foundry::ts_rs\")]"));
        assert!(!output.contains("   Clone,"));
        assert!(output.contains("serde::Serialize"));
        assert!(output.contains("foundry::ts_rs::TS"));
        assert!(output.contains("foundry::ApiSchema"));
        assert!(output.contains("foundry::Model"));
    }

    #[test]
    fn render_ids_template_contains_typed_id_ssot_enums() {
        let output = render_ids_template();
        assert!(output.contains("use foundry::prelude::*"));
        assert!(output.contains("#[derive(Clone, Copy, FoundryId)]"));
        assert!(output.contains("#[foundry(id = GuardId, rename_all = \"snake_case\")]"));
        assert!(output.contains("pub enum AuthGuard"));
        assert!(output.contains("Api,"));
        assert!(output.contains("#[derive(Clone, Copy, Debug, PartialEq, Eq, AppEnum)]"));
        assert!(output.contains("#[foundry(id = \"ability\", id_type = PermissionId)]"));
        assert!(output.contains("#[foundry(key = \"dashboard:view\")]"));
        assert!(output.contains("pub enum Ability"));
        assert!(output.contains("#[foundry(id = RouteId)]"));
        assert!(output.contains("pub enum Route"));
        assert!(!output.contains("impl From<"));
        assert!(!output.contains("TODO"));
    }

    #[test]
    fn render_request_template_contains_validated_exportable_dto() {
        let output = render_request_template("StorePostRequest");
        assert!(output.contains("pub struct StorePostRequest {"));
        assert!(output.contains("serde::Deserialize"));
        assert!(output.contains("foundry::ts_rs::TS"));
        assert!(output.contains("foundry::ApiSchema"));
        assert!(output.contains("foundry::Validate"));
        assert!(output.contains("#[ts(crate = \"foundry::ts_rs\")]"));
        assert!(output.contains("#[validate(required)]"));
        assert!(output.contains("pub name: String"));
        assert!(!output.contains("use foundry::prelude::*"));
        assert!(!output.contains("TODO"));
    }

    #[test]
    fn render_response_template_contains_exportable_dto() {
        let output = render_response_template("PostResponse");
        assert!(output.contains("pub struct PostResponse {"));
        assert!(output.contains("serde::Serialize"));
        assert!(output.contains("foundry::ts_rs::TS"));
        assert!(output.contains("foundry::ApiSchema"));
        assert!(output.contains("#[ts(crate = \"foundry::ts_rs\")]"));
        assert!(output.contains("pub message: String"));
        assert!(!output.contains("use foundry::prelude::*"));
        assert!(!output.contains("TODO"));
    }

    #[test]
    fn render_guard_template_contains_registerable_guard() {
        let output = render_guard_template("ApiGuard", "api", "API");
        assert!(output.contains("pub const API_GUARD: GuardId"));
        assert!(output.contains("GuardId::new(\"api\")"));
        assert!(output.contains("pub struct ApiGuard;"));
        assert!(output.contains("pub fn register(registrar: &mut ServiceRegistrar)"));
        assert!(output.contains("registrar.register_guard(API_GUARD, ApiGuard)"));
        assert!(output.contains("#[foundry::async_trait]"));
        assert!(output.contains("impl BearerAuthenticator for ApiGuard"));
        assert!(output.contains("Ok(None)"));
        assert!(!output.contains("use async_trait::async_trait"));
        assert!(!output.contains("TODO"));
    }

    #[test]
    fn render_policy_template_contains_registerable_policy() {
        let output = render_policy_template("CanEditPost", "can_edit_post", "CAN_EDIT_POST");
        assert!(output.contains("pub const CAN_EDIT_POST_POLICY: PolicyId"));
        assert!(output.contains("PolicyId::new(\"can_edit_post\")"));
        assert!(output.contains("pub struct CanEditPost;"));
        assert!(output.contains("pub fn register(registrar: &mut ServiceRegistrar)"));
        assert!(output.contains("registrar.register_policy(CAN_EDIT_POST_POLICY, CanEditPost)"));
        assert!(output.contains("#[foundry::async_trait]"));
        assert!(output.contains("impl Policy for CanEditPost"));
        assert!(output.contains("Ok(false)"));
        assert!(!output.contains("use async_trait::async_trait"));
        assert!(!output.contains("TODO"));
    }

    #[test]
    fn async_scaffolds_use_foundry_async_trait_reexport() {
        for output in [
            render_migration_template(),
            render_seeder_template(),
            render_guard_template("ApiGuard", "api", "API"),
            render_job_template(
                "SendWelcomeEmail",
                "send_welcome_email",
                "SEND_WELCOME_EMAIL",
            ),
        ] {
            assert!(output.contains("#[foundry::async_trait]"));
            assert!(!output.contains("use async_trait::async_trait"));
        }
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
        assert!(output.contains("serde::Serialize"));
        assert!(output.contains("serde::Deserialize"));
        assert!(output.contains("foundry::ts_rs::TS"));
        assert!(output.contains("foundry::TS"));
        assert!(output.contains("#[ts(crate = \"foundry::ts_rs\")]"));
        assert!(output.contains("#[foundry::async_trait]"));
        assert!(output.contains("const ID: JobId = SEND_WELCOME_EMAIL_JOB;"));
        assert!(!output.contains("TODO"));
    }

    #[test]
    fn render_event_template_contains_exportable_payload_registration() {
        let output = render_event_template("OrderPlaced", "order.placed", "ORDER_PLACED");
        assert!(output.contains("pub const ORDER_PLACED_EVENT: EventId"));
        assert!(output.contains("EventId::new(\"order.placed\")"));
        assert!(output.contains("pub struct OrderPlaced {}"));
        assert!(output.contains("serde::Serialize"));
        assert!(output.contains("serde::Deserialize"));
        assert!(output.contains("foundry::ts_rs::TS"));
        assert!(output.contains("foundry::TS"));
        assert!(output.contains("foundry::ApiSchema"));
        assert!(output.contains("#[ts(crate = \"foundry::ts_rs\")]"));
        assert!(output.contains("impl Event for OrderPlaced"));
        assert!(output.contains("TsEventPayload::new(ORDER_PLACED_EVENT, \"OrderPlaced\")"));
        assert!(!output.contains("TODO"));
    }

    #[test]
    fn render_notification_template_contains_typed_manifest_registration() {
        let output = render_notification_template("OrderShipped", "order.shipped", "ORDER_SHIPPED");
        assert!(output.contains("pub const ORDER_SHIPPED_NOTIFICATION_TYPE: &str"));
        assert!(output.contains("\"order.shipped\""));
        assert!(output.contains("pub struct OrderShipped {}"));
        assert!(output.contains("serde::Serialize"));
        assert!(output.contains("serde::Deserialize"));
        assert!(output.contains("foundry::ts_rs::TS"));
        assert!(output.contains("foundry::TS"));
        assert!(output.contains("foundry::ApiSchema"));
        assert!(output.contains("#[ts(crate = \"foundry::ts_rs\")]"));
        assert!(output.contains("impl Notification for OrderShipped"));
        assert!(output.contains("vec![NOTIFY_DATABASE, NOTIFY_BROADCAST]"));
        assert!(output.contains("foundry::serde_json::to_value(self).ok()"));
        assert!(output.contains("TsNotification {"));
        assert!(output.contains("notification_type: ORDER_SHIPPED_NOTIFICATION_TYPE"));
        assert!(output.contains("payload: \"OrderShipped\""));
        assert!(!output.contains("TODO"));
    }

    #[test]
    fn render_command_template_contains_register_without_placeholder_todo() {
        let output = render_command_template("SyncInventory", "sync_inventory", "SYNC_INVENTORY");
        assert!(output.contains("pub const SYNC_INVENTORY_COMMAND: CommandId"));
        assert!(output.contains("pub fn register("));
        assert!(output.contains("Command::new(\"sync_inventory\")"));
        assert!(!output.contains("clap::Command"));
        assert!(!output.contains("TODO"));
    }
}
