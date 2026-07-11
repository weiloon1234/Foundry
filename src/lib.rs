extern crate self as foundry;

#[doc(hidden)]
pub mod __reexports {
    pub use async_trait::async_trait;
}

#[doc(hidden)]
pub mod __private {
    use std::path::PathBuf;

    use crate::database::lifecycle::GeneratedDatabasePaths;
    use crate::database::{MigrationFile, SeederFile};
    use crate::foundation::{Result, ServiceRegistrar};
    use crate::support::{MigrationId, SeederId};

    pub fn register_generated_database_paths(
        registrar: &ServiceRegistrar,
        migration_dirs: Vec<PathBuf>,
        seeder_dirs: Vec<PathBuf>,
    ) -> Result<()> {
        registrar.singleton(GeneratedDatabasePaths::new(migration_dirs, seeder_dirs))
    }

    pub fn register_generated_migration_file<M>(
        registrar: &ServiceRegistrar,
        id: MigrationId,
    ) -> Result<()>
    where
        M: MigrationFile,
    {
        registrar.register_generated_migration_file::<M>(id)
    }

    pub fn register_generated_seeder_file<S>(
        registrar: &ServiceRegistrar,
        id: SeederId,
    ) -> Result<()>
    where
        S: SeederFile,
    {
        registrar.register_generated_seeder_file::<S>(id)
    }
}

#[macro_export]
macro_rules! register_generated_database {
    ($registrar:expr) => {{
        mod __foundry_generated_database {
            include!(concat!(env!("OUT_DIR"), "/foundry_database_generated.rs"));
        }

        __foundry_generated_database::register($registrar)
    }};
}

pub mod app_enum;
pub mod attachments;
pub mod audit;
pub mod auth;
pub mod cache;
pub mod cli;
pub mod config;
pub mod contract;
pub mod countries;
pub mod database;
pub mod datatable;
pub mod email;
pub mod events;
pub mod foundation;
pub mod http;
pub mod http_client;
pub mod i18n;
pub mod imaging;
pub mod jobs;
pub mod kernel;
pub mod logging;
pub mod metadata;
pub mod notifications;
pub mod openapi;
pub mod plugin;
pub mod prelude;
pub mod public;
pub mod redis;
pub mod scheduler;
pub mod settings;
pub mod storage;
pub mod support;
pub mod testing;
pub mod translations;
pub mod typescript;
pub mod validation;
pub mod websocket;

pub use public::*;
