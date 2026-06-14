use std::path::Path;
use std::sync::Arc;

use crate::cli::CommandRegistrar;
use crate::foundation::Error;
use crate::support::generated_manifest::{
    ensure_generated_file_writable, generated_file_exists_without_symlink, write_generated_file,
};
use crate::support::CommandId;

const ENV_PUBLISH_COMMAND: CommandId = CommandId::new("env:publish");

pub(crate) fn env_publish_cli_registrar() -> CommandRegistrar {
    Arc::new(|registry| {
        registry.command(
            ENV_PUBLISH_COMMAND,
            clap::Command::new(ENV_PUBLISH_COMMAND.as_str().to_string())
                .about("Publish a .env.example file with all available environment variables")
                .arg(
                    clap::Arg::new("path")
                        .long("path")
                        .value_name("DIR")
                        .default_value(".")
                        .help("Directory to write the .env.example file to"),
                )
                .arg(
                    clap::Arg::new("force")
                        .long("force")
                        .action(clap::ArgAction::SetTrue)
                        .help("Overwrite existing .env.example file"),
                ),
            |invocation| async move {
                let dir = invocation
                    .matches()
                    .get_one::<String>("path")
                    .map(|s| s.as_str())
                    .unwrap_or(".");
                let force = invocation.matches().get_flag("force");

                let path = Path::new(dir);
                if !path.exists() {
                    std::fs::create_dir_all(path).map_err(Error::other)?;
                }

                let file_path = path.join(".env.example");
                if let Err(error) = ensure_generated_file_writable(&file_path, force) {
                    if !force && generated_file_exists_without_symlink(&file_path) {
                        println!(
                            ".env.example already exists at {}. Use --force to overwrite.",
                            file_path.display()
                        );
                        return Ok(());
                    }
                    return Err(error);
                }

                write_generated_file(&file_path, sample_env())?;
                println!("Published .env.example to {}", file_path.display());

                Ok(())
            },
        )?;
        Ok(())
    })
}

fn sample_env() -> String {
    super::published::render_sample_env()
}
