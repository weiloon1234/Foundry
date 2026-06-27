use std::future::Future;
use std::pin::Pin;

mod blocking;
mod collection;
mod crypt;
mod datetime;
pub(crate) mod dotted_ids;
pub(crate) mod filename;
pub(crate) mod generated_manifest;
mod hash;
pub(crate) mod hmac;
mod identifiers;
pub(crate) mod javascript;
pub mod lock;
pub(crate) mod redaction;
pub(crate) mod runtime;
mod sanitize;
pub(crate) mod sha256;
pub(crate) mod sync;
mod token;

pub use blocking::run_blocking;
pub use collection::Collection;
pub use crypt::CryptManager;
pub use datetime::{Clock, Date, DateTime, LocalDateTime, Time, Timezone};
pub use hash::HashManager;
pub use sanitize::{sanitize_html, strip_tags};
pub use sha256::{sha256_hex, sha256_hex_str};
pub use token::Token;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

pub use identifiers::{
    ChannelEventId, ChannelId, CommandId, EventId, GuardId, JobId, MigrationId, ModelId,
    NotificationChannelId, PermissionId, PluginAssetId, PluginId, PluginScaffoldId, PolicyId,
    ProbeId, QueueId, RoleId, RouteId, ScheduleId, SeederId, ValidationRuleId,
};

pub fn boxed<F, T>(future: F) -> BoxFuture<T>
where
    F: Future<Output = T> + Send + 'static,
{
    Box::pin(future)
}
