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

    println!("wrote {}", migration_path.display());
    println!("next: rebuild the app before running db:migrate so Foundry discovers it");
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

    println!("wrote {}", seeder_path.display());
    println!("next: rebuild the app before running db:seed so Foundry discovers it");
    Ok(())
}

async fn make_model_command(invocation: CommandInvocation) -> Result<()> {
    let name = required_name(invocation.matches())?;
    let pascal = to_pascal_case(name);
    let snake = to_snake_case(name);
    let model_dir = resolve_output_dir(invocation.matches(), "src/app/models")?;
    let filename = format!("{snake}.rs");
    let model_path = write_scaffold_file(
        &model_dir,
        &filename,
        render_model_template(&pascal, &snake),
        invocation.matches().get_flag("force"),
    )?;

    println!("wrote {}", model_path.display());
    println!("next: add fields, create a migration, and register the module from your app");
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

    println!("wrote {}", job_path.display());
    println!("next: implement handle(), then register the job in a service provider");
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

fn render_model_template(pascal: &str, snake: &str) -> String {
    let table_name = format!("{snake}s");
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

#[cfg(test)]
mod tests {
    use super::{
        render_command_template, render_job_template, render_model_template, to_pascal_case,
        to_screaming_snake_case, write_scaffold_file,
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
    fn render_model_template_contains_struct() {
        let output = render_model_template("User", "user");
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
