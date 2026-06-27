use super::types::{EnumKey, EnumKeyKind, EnumMeta, EnumOption};
use crate::database::DbType;
use crate::support::Collection;

pub trait FoundryAppEnum: Sized + Clone + Send + Sync + 'static {
    /// The database type this enum stores as.
    /// Text for string-backed, Int32 for int-backed.
    const DB_TYPE: DbType;

    /// The enum identifier used for metadata/export grouping.
    /// Defaults to the type name in normalized snake_case and can be overridden
    /// with `#[foundry(id = "...")]`.
    fn id() -> &'static str;

    /// Get the stored key for this variant.
    fn key(self) -> EnumKey;

    /// All valid keys for this enum.
    fn keys() -> Collection<EnumKey>;

    /// All accepted string inputs for this enum, including canonical keys and
    /// compatibility aliases.
    fn accepted_keys() -> Collection<String> {
        let mut keys = Vec::new();
        for option in Self::options() {
            match option.value {
                EnumKey::String(value) => keys.push(value),
                EnumKey::Int(value) => keys.push(value.to_string()),
            }
            keys.extend(option.aliases);
        }
        Collection::from(keys)
    }

    /// Parse a string key into the enum variant.
    /// For string-backed: matches against stored string keys.
    /// For int-backed: parses string as i32, then matches discriminants.
    /// Also matches any declared aliases.
    fn parse_key(key: &str) -> Option<Self>;

    /// Get the translation key metadata for this variant.
    ///
    /// Default format is `enum.{enum_id}.{variant_snake_case}`.
    /// `#[foundry(label_prefix = "...")]` changes the prefix for the whole enum.
    /// `#[foundry(label_key = "...")]` overrides a single variant verbatim.
    fn label_key(self) -> &'static str;

    /// All options as stored value + label key pairs.
    fn options() -> Collection<EnumOption>;

    /// Full metadata for this enum. This is the canonical runtime/export source.
    fn meta() -> EnumMeta;

    /// The key kind (String or Int).
    fn key_kind() -> EnumKeyKind;
}
