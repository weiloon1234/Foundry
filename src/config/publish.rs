use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cli::CommandRegistrar;
use crate::foundation::{Error, Result};
use crate::support::generated_manifest::{
    ensure_generated_file_writable, generated_file_exists_without_symlink, write_generated_file,
};
use crate::support::CommandId;

const CONFIG_PUBLISH_COMMAND: CommandId = CommandId::new("config:publish");
const KEY_GENERATE_COMMAND: CommandId = CommandId::new("key:generate");
const MIGRATE_PUBLISH_COMMAND: CommandId = CommandId::new("migrate:publish");
const SEED_PUBLISH_COMMAND: CommandId = CommandId::new("seed:publish");
const SEED_COUNTRIES_COMMAND: CommandId = CommandId::new("seed:countries");
const ABOUT_COMMAND: CommandId = CommandId::new("about");

/// Generate the full sample configuration TOML.
///
/// Required fields are uncommented; optional fields are commented out with
/// their default values so users can uncomment what they need.
pub fn sample_config() -> String {
    super::published::render_sample_config()
}

/// Generate the preferred split sample configuration TOML files.
pub fn sample_config_files() -> Vec<(&'static str, String)> {
    super::published::render_sample_config_files()
}

enum ConfigPublishOutcome {
    Written(Vec<PathBuf>),
    Exists(PathBuf),
}

pub(crate) fn config_publish_cli_registrar() -> CommandRegistrar {
    Arc::new(|registry| {
        registry.command(
            CONFIG_PUBLISH_COMMAND,
            clap::Command::new(CONFIG_PUBLISH_COMMAND.as_str().to_string())
                .about("Publish sample configuration files to the config directory")
                .arg(
                    clap::Arg::new("path")
                        .long("path")
                        .value_name("DIR")
                        .default_value("config")
                        .help("Directory to write configuration files to"),
                )
                .arg(
                    clap::Arg::new("single-file")
                        .long("single-file")
                        .action(clap::ArgAction::SetTrue)
                        .help("Write the legacy config/foundry.toml file instead of split files"),
                )
                .arg(
                    clap::Arg::new("force")
                        .long("force")
                        .action(clap::ArgAction::SetTrue)
                        .help("Overwrite existing config file"),
                ),
            |invocation| async move {
                let dir = invocation
                    .matches()
                    .get_one::<String>("path")
                    .map(|s| s.as_str())
                    .unwrap_or("config");
                let force = invocation.matches().get_flag("force");
                let single_file = invocation.matches().get_flag("single-file");

                let path = Path::new(dir);
                match publish_sample_config(path, force, single_file)? {
                    ConfigPublishOutcome::Written(files) => {
                        if single_file {
                            if let Some(file) = files.first() {
                                println!("Configuration published to {}", file.display());
                            }
                        } else {
                            println!("Configuration files published to {}", path.display());
                            for file in files {
                                println!("  {}", file.display());
                            }
                        }
                    }
                    ConfigPublishOutcome::Exists(file) => {
                        println!(
                            "Config file already exists at {}. Use --force to overwrite.",
                            file.display()
                        );
                    }
                }

                Ok(())
            },
        )?;

        registry.command(
            KEY_GENERATE_COMMAND,
            clap::Command::new(KEY_GENERATE_COMMAND.as_str().to_string())
                .about("Generate application keys (signing key and encryption key)"),
            |_invocation| async move {
                use base64::{engine::general_purpose::STANDARD, Engine};

                let signing_key = STANDARD.encode(crate::support::Token::bytes(32)?);
                let crypt_key = STANDARD.encode(crate::support::Token::bytes(32)?);

                println!("Keys generated successfully.\n");
                println!("Add to your config file:\n");
                println!("  [app]");
                println!("  signing_key = \"{signing_key}\"\n");
                println!("  [crypt]");
                println!("  key = \"{crypt_key}\"\n");
                println!("Or set via environment variables:\n");
                println!("  APP__SIGNING_KEY={signing_key}");
                println!("  CRYPT__KEY={crypt_key}");

                Ok(())
            },
        )?;

        registry.command(
            MIGRATE_PUBLISH_COMMAND,
            clap::Command::new(MIGRATE_PUBLISH_COMMAND.as_str().to_string())
                .about("Publish framework migration files to your project")
                .arg(
                    clap::Arg::new("path")
                        .long("path")
                        .value_name("DIR")
                        .default_value("database/migrations")
                        .help("Directory to write migration files to"),
                )
                .arg(
                    clap::Arg::new("force")
                        .long("force")
                        .action(clap::ArgAction::SetTrue)
                        .help("Overwrite existing migration files"),
                ),
            |invocation| async move {
                let dir = invocation
                    .matches()
                    .get_one::<String>("path")
                    .map(|s| s.as_str())
                    .unwrap_or("database/migrations");
                let force = invocation.matches().get_flag("force");

                publish_framework_files(dir, FRAMEWORK_MIGRATIONS, force, "migration")?;
                Ok(())
            },
        )?;

        registry.command(
            SEED_PUBLISH_COMMAND,
            clap::Command::new(SEED_PUBLISH_COMMAND.as_str().to_string())
                .about("Publish framework seeder files to your project")
                .arg(
                    clap::Arg::new("path")
                        .long("path")
                        .value_name("DIR")
                        .default_value("database/seeders")
                        .help("Directory to write seeder files to"),
                )
                .arg(
                    clap::Arg::new("force")
                        .long("force")
                        .action(clap::ArgAction::SetTrue)
                        .help("Overwrite existing seeder files"),
                ),
            |invocation| async move {
                let dir = invocation
                    .matches()
                    .get_one::<String>("path")
                    .map(|s| s.as_str())
                    .unwrap_or("database/seeders");
                let force = invocation.matches().get_flag("force");

                publish_framework_files(dir, FRAMEWORK_SEEDERS, force, "seeder")?;
                Ok(())
            },
        )?;

        registry.command(
            SEED_COUNTRIES_COMMAND,
            clap::Command::new(SEED_COUNTRIES_COMMAND.as_str().to_string())
                .about("Seed the countries table with 250 built-in country records"),
            |invocation| async move {
                let app = invocation.app();
                let count = crate::countries::seed_countries(app).await?;
                println!("Seeded {count} countries.");
                Ok(())
            },
        )?;

        registry.command(
            ABOUT_COMMAND,
            clap::Command::new(ABOUT_COMMAND.as_str().to_string())
                .about("Display framework version and environment summary"),
            |invocation| async move {
                let app = invocation.app();
                let config = app.config();

                println!("Foundry Framework v{}\n", env!("CARGO_PKG_VERSION"));

                let app_config = config.app().unwrap_or_default();
                println!("  Environment:  {}", app_config.environment);
                println!("  Timezone:     {}", app_config.timezone);

                let signing = if app_config.signing_key.is_empty() {
                    "not configured"
                } else if app_config.signing_key_bytes().is_ok() {
                    "configured"
                } else {
                    "invalid"
                };
                println!("  Signing key:  {}", signing);

                if let Ok(db) = config.database() {
                    let db_status = if db.url.is_empty() {
                        "not configured"
                    } else {
                        "configured"
                    };
                    println!("  Database:     {}", db_status);
                    if db.read_url.as_deref().is_some_and(|u| !u.is_empty()) {
                        println!("  Read replica: configured");
                    }
                }

                if let Ok(redis) = config.redis() {
                    let redis_status = if redis.url.is_empty() {
                        "not configured"
                    } else {
                        "configured"
                    };
                    println!("  Redis:        {}", redis_status);
                }

                if let Ok(cache) = config.cache() {
                    println!("  Cache:        {:?}", cache.driver);
                }

                if let Ok(logging) = config.logging() {
                    println!("  Log level:    {:?}", logging.level);
                    println!("  Log format:   {:?}", logging.format);
                    println!("  Retention:    {} days", logging.retention_days);
                }

                if let Ok(plugins) = app.resolve::<crate::plugin::PluginRegistry>() {
                    if !plugins.is_empty() {
                        println!("  Plugins:      registered");
                    }
                }

                Ok(())
            },
        )?;

        Ok(())
    })
}

fn publish_sample_config(
    path: &Path,
    force: bool,
    single_file: bool,
) -> Result<ConfigPublishOutcome> {
    if !path.exists() {
        std::fs::create_dir_all(path).map_err(Error::other)?;
    }

    let files = if single_file {
        vec![("foundry.toml", sample_config())]
    } else {
        sample_config_files()
    };

    for (filename, _) in &files {
        let file_path = path.join(filename);
        if let Err(error) = ensure_generated_file_writable(&file_path, force) {
            if !force && generated_file_exists_without_symlink(&file_path) {
                return Ok(ConfigPublishOutcome::Exists(file_path));
            }
            return Err(error);
        }
    }

    let mut written = Vec::with_capacity(files.len());
    for (filename, contents) in files {
        let file_path = path.join(filename);
        write_generated_file(&file_path, contents)?;
        written.push(file_path);
    }

    Ok(ConfigPublishOutcome::Written(written))
}

fn publish_framework_files(
    dir: &str,
    files: &[(&'static str, &'static str)],
    force: bool,
    kind: &str,
) -> Result<()> {
    let path = Path::new(dir);
    if !path.exists() {
        std::fs::create_dir_all(path).map_err(Error::other)?;
    }

    let mut published = 0;
    for (name, contents) in files {
        let file_path = path.join(name);
        if let Err(error) = ensure_generated_file_writable(&file_path, force) {
            if !force && generated_file_exists_without_symlink(&file_path) {
                println!("  skip  {} (exists)", name);
                continue;
            }
            return Err(error);
        }

        write_generated_file(&file_path, contents)?;
        println!("  create  {}", name);
        published += 1;
    }

    if published == 0 {
        println!("\nAll {kind}s already exist. Use --force to overwrite.");
    } else {
        println!("\n{published} {kind}(s) published to {dir}");
    }

    Ok(())
}

#[cfg(test)]
mod config_publish_tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{publish_sample_config, ConfigPublishOutcome};

    #[test]
    fn config_publish_defaults_to_split_files() {
        let directory = tempdir().unwrap();
        let outcome = publish_sample_config(directory.path(), false, false).unwrap();

        let ConfigPublishOutcome::Written(files) = outcome else {
            panic!("expected split config files to be written");
        };

        assert!(files.iter().any(|path| path.ends_with("00-app.toml")));
        assert!(files.iter().any(|path| path.ends_with("10-http.toml")));
        assert!(files.iter().any(|path| path.ends_with("70-storage.toml")));
        assert!(!directory.path().join("foundry.toml").exists());
    }

    #[test]
    fn config_publish_single_file_keeps_legacy_foundry_toml() {
        let directory = tempdir().unwrap();
        let outcome = publish_sample_config(directory.path(), false, true).unwrap();

        let ConfigPublishOutcome::Written(files) = outcome else {
            panic!("expected legacy config file to be written");
        };

        assert_eq!(files.len(), 1);
        assert!(directory.path().join("foundry.toml").exists());
        assert!(!directory.path().join("00-app.toml").exists());
    }

    #[test]
    fn config_publish_refuses_existing_split_file_without_force() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join("00-app.toml"), "existing").unwrap();

        let outcome = publish_sample_config(directory.path(), false, false).unwrap();

        let ConfigPublishOutcome::Exists(path) = outcome else {
            panic!("expected existing file outcome");
        };
        assert!(path.ends_with("00-app.toml"));
    }
}

/// Framework-provided migration files (Rust format, discoverable by foundry-build).
const FRAMEWORK_MIGRATIONS: &[(&str, &str)] = &[
    (
        "000000000000_create_database_primitives.rs",
        include_str!("../../database/migrations/000000000000_create_database_primitives.rs"),
    ),
    (
        "000000000001_create_personal_access_tokens.rs",
        include_str!("../../database/migrations/000000000001_create_personal_access_tokens.rs"),
    ),
    (
        "000000000002_create_password_reset_tokens.rs",
        include_str!("../../database/migrations/000000000002_create_password_reset_tokens.rs"),
    ),
    (
        "000000000003_create_notifications.rs",
        include_str!("../../database/migrations/000000000003_create_notifications.rs"),
    ),
    (
        "000000000004_create_job_history.rs",
        include_str!("../../database/migrations/000000000004_create_job_history.rs"),
    ),
    (
        "000000000005_create_attachments.rs",
        include_str!("../../database/migrations/000000000005_create_attachments.rs"),
    ),
    (
        "000000000006_create_metadata.rs",
        include_str!("../../database/migrations/000000000006_create_metadata.rs"),
    ),
    (
        "000000000007_create_model_translations.rs",
        include_str!("../../database/migrations/000000000007_create_model_translations.rs"),
    ),
    (
        "000000000008_create_countries.rs",
        include_str!("../../database/migrations/000000000008_create_countries.rs"),
    ),
    (
        "000000000009_create_settings.rs",
        include_str!("../../database/migrations/000000000009_create_settings.rs"),
    ),
    (
        "000000000010_create_audit_logs.rs",
        include_str!("../../database/migrations/000000000010_create_audit_logs.rs"),
    ),
    (
        "000000000011_create_auth_mfa_totp_factors.rs",
        include_str!("../../database/migrations/000000000011_create_auth_mfa_totp_factors.rs"),
    ),
    (
        "000000000012_index_job_history_created_at.rs",
        include_str!("../../database/migrations/000000000012_index_job_history_created_at.rs"),
    ),
];

/// Framework-provided seeder files (Rust format, discoverable by foundry-build).
const FRAMEWORK_SEEDERS: &[(&str, &str)] = &[(
    "000000000001_countries_seeder.rs",
    include_str!("../../database/seeders/000000000001_countries_seeder.rs"),
)];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    use super::FRAMEWORK_MIGRATIONS;

    #[test]
    fn framework_migration_manifest_covers_all_framework_migration_files() {
        let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("database/migrations");
        let files = fs::read_dir(migrations_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".rs"))
            .collect::<BTreeSet<_>>();
        let published = FRAMEWORK_MIGRATIONS
            .iter()
            .map(|(name, _)| (*name).to_string())
            .collect::<BTreeSet<_>>();

        assert_eq!(published, files);
    }
}
