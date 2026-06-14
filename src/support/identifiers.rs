use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::Deref;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

macro_rules! typed_identifier {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Cow<'static, str>);

        impl $name {
            pub const fn new(value: &'static str) -> Self {
                Self(Cow::Borrowed(value))
            }

            pub fn owned(value: impl Into<String>) -> Self {
                Self(Cow::Owned(value.into()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Deref for $name {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

typed_identifier!(GuardId);
typed_identifier!(PolicyId);
typed_identifier!(PermissionId);
typed_identifier!(RoleId);
typed_identifier!(RouteId);
typed_identifier!(ValidationRuleId);
typed_identifier!(ChannelId);
typed_identifier!(ChannelEventId);
typed_identifier!(JobId);
typed_identifier!(QueueId);
typed_identifier!(EventId);
typed_identifier!(CommandId);
typed_identifier!(ScheduleId);
typed_identifier!(ProbeId);
typed_identifier!(PluginId);
typed_identifier!(PluginAssetId);
typed_identifier!(PluginScaffoldId);
typed_identifier!(MigrationId);
typed_identifier!(SeederId);
typed_identifier!(NotificationChannelId);

pub struct ModelId<M> {
    value: Uuid,
    _marker: PhantomData<fn() -> M>,
}

impl<M> ModelId<M> {
    pub fn generate() -> Self {
        Self::from_uuid(Uuid::now_v7())
    }

    pub const fn from_uuid(value: Uuid) -> Self {
        Self {
            value,
            _marker: PhantomData,
        }
    }

    pub fn parse_str(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self::from_uuid)
    }

    pub const fn as_uuid(&self) -> &Uuid {
        &self.value
    }

    pub const fn into_uuid(self) -> Uuid {
        self.value
    }
}

impl<M> Clone for ModelId<M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M> Copy for ModelId<M> {}

impl<M> PartialEq for ModelId<M> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<M> Eq for ModelId<M> {}

impl<M> PartialOrd for ModelId<M> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<M> Ord for ModelId<M> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.value.cmp(&other.value)
    }
}

impl<M> Hash for ModelId<M> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl<M> fmt::Debug for ModelId<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("ModelId").field(&self.value).finish()
    }
}

impl<M> fmt::Display for ModelId<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.value, formatter)
    }
}

impl<M> FromStr for ModelId<M> {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse_str(value)
    }
}

impl<M> Serialize for ModelId<M> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.value.to_string())
    }
}

impl<'de, M> Deserialize<'de> for ModelId<M> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse_str(&value).map_err(serde::de::Error::custom)
    }
}

impl<M> From<Uuid> for ModelId<M> {
    fn from(value: Uuid) -> Self {
        Self::from_uuid(value)
    }
}

impl<M> From<ModelId<M>> for Uuid {
    fn from(value: ModelId<M>) -> Self {
        value.into_uuid()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChannelId, GuardId, MigrationId, ModelId, PluginAssetId, PluginId, PluginScaffoldId,
        ProbeId, QueueId, RouteId, SeederId,
    };
    use uuid::Uuid;

    struct User;
    struct Order;

    #[test]
    fn identifiers_expose_static_values() {
        const API: GuardId = GuardId::new("api");
        const CHAT: ChannelId = ChannelId::new("chat");
        const READINESS: ProbeId = ProbeId::new("ready.database");
        const DEFAULT_QUEUE: QueueId = QueueId::new("default");
        const ROUTE: RouteId = RouteId::new("users.show");
        const PLUGIN: PluginId = PluginId::new("foundry.plugin");
        const ASSET: PluginAssetId = PluginAssetId::new("config");
        const SCAFFOLD: PluginScaffoldId = PluginScaffoldId::new("dashboard");
        const MIGRATION: MigrationId = MigrationId::new("202604091200_create_users");
        const SEEDER: SeederId = SeederId::new("users.seed");

        assert_eq!(API.as_str(), "api");
        assert_eq!(CHAT.as_str(), "chat");
        assert_eq!(READINESS.as_str(), "ready.database");
        assert_eq!(DEFAULT_QUEUE.as_str(), "default");
        assert_eq!(ROUTE.as_str(), "users.show");
        assert_eq!(PLUGIN.as_str(), "foundry.plugin");
        assert_eq!(ASSET.as_str(), "config");
        assert_eq!(SCAFFOLD.as_str(), "dashboard");
        assert_eq!(MIGRATION.as_str(), "202604091200_create_users");
        assert_eq!(SEEDER.as_str(), "users.seed");
    }

    #[test]
    fn model_ids_round_trip_as_strings() {
        let parsed = ModelId::<User>::parse_str("018f05a0-6d2d-7af4-977d-2a5d76b95f0c").unwrap();
        let json = serde_json::to_string(&parsed).unwrap();
        let reparsed: ModelId<User> = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed, reparsed);
        assert_eq!(parsed.to_string(), "018f05a0-6d2d-7af4-977d-2a5d76b95f0c");
    }

    #[test]
    fn model_ids_preserve_type_and_uuid_access() {
        let uuid = Uuid::parse_str("018f05a0-6d2d-7af4-977d-2a5d76b95f0c").unwrap();
        let user_id = ModelId::<User>::from_uuid(uuid);
        let order_id = ModelId::<Order>::from_uuid(uuid);

        fn takes_user_id(_: ModelId<User>) {}

        takes_user_id(user_id);
        assert_eq!(user_id.as_uuid(), &uuid);
        assert_eq!(Uuid::from(order_id), uuid);
    }
}
