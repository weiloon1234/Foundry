mod helpers;
mod r#trait;
mod types;

pub use helpers::{to_snake_case, to_title_text};
pub use r#trait::FoundryAppEnum;
pub use types::{EnumKey, EnumKeyKind, EnumMeta, EnumOption};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{DbType, DbValue, FromDbValue, ToDbValue};
    use crate::validation::{RuleRegistry, Validator};
    use crate::{config::ConfigRepository, foundation::Container};

    // -----------------------------------------------------------------------
    // Test enums
    // -----------------------------------------------------------------------

    #[derive(Clone, Debug, PartialEq, Eq, foundry::AppEnum)]
    enum OrderStatus {
        Pending,
        Reviewing,
        Completed,
    }

    #[derive(Clone, Debug, PartialEq, Eq, foundry::AppEnum)]
    enum OrderStatusWithOverrides {
        Pending,
        #[foundry(key = "in_review")]
        Reviewing,
        #[foundry(label_key = "Order completed")]
        Completed,
    }

    #[derive(Clone, Debug, PartialEq, Eq, foundry::AppEnum)]
    enum UserStatus {
        Pending = 0,
        Verified = 1,
        Suspended = 2,
    }

    #[derive(Clone, Debug, PartialEq, Eq, foundry::AppEnum)]
    #[foundry(id = "custom_status")]
    enum CustomIdEnum {
        Alpha,
        Beta,
    }

    #[derive(Clone, Debug, PartialEq, Eq, foundry::AppEnum)]
    enum AliasedEnum {
        #[foundry(aliases = ["awaiting", "queued"])]
        Pending,
        Active,
    }

    #[derive(Clone, Debug, PartialEq, Eq, foundry::AppEnum)]
    #[foundry(label_prefix = "admin.credits.transaction_types")]
    enum CreditTransactionType {
        AdminAdd,
        AdminDeduct,
        TransferReceived,
        TransferSent,
    }

    #[derive(Clone, Debug, PartialEq, Eq, foundry::AppEnum)]
    enum MixedIdentifierStatus {
        Credit1,
        HTTP2Enabled,
    }

    #[derive(Clone, Debug, PartialEq, Eq, foundry::AppEnum)]
    enum HTTP2Setting {
        Enabled,
        Disabled,
    }

    fn test_app() -> crate::foundation::AppContext {
        crate::foundation::AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap()
    }

    // -----------------------------------------------------------------------
    // String-backed tests
    // -----------------------------------------------------------------------

    #[test]
    fn string_backed_key_returns_snake_case() {
        assert_eq!(
            OrderStatus::Pending.key(),
            EnumKey::String("pending".into())
        );
        assert_eq!(
            OrderStatus::Reviewing.key(),
            EnumKey::String("reviewing".into())
        );
        assert_eq!(
            OrderStatus::Completed.key(),
            EnumKey::String("completed".into())
        );
    }

    #[test]
    fn string_backed_parse_key_valid() {
        assert_eq!(
            OrderStatus::parse_key("pending"),
            Some(OrderStatus::Pending)
        );
        assert_eq!(
            OrderStatus::parse_key("reviewing"),
            Some(OrderStatus::Reviewing)
        );
        assert_eq!(
            OrderStatus::parse_key("completed"),
            Some(OrderStatus::Completed)
        );
    }

    #[test]
    fn string_backed_parse_key_invalid() {
        assert_eq!(OrderStatus::parse_key("unknown"), None);
    }

    #[test]
    fn string_backed_from_str_uses_parse_key() {
        assert_eq!("pending".parse::<OrderStatus>(), Ok(OrderStatus::Pending));
        assert!("unknown".parse::<OrderStatus>().is_err());
    }

    #[test]
    fn string_backed_keys_returns_all() {
        let keys = OrderStatus::keys();
        assert_eq!(keys.len(), 3);
    }

    #[test]
    fn string_backed_label_key_default() {
        assert_eq!(
            OrderStatus::Pending.label_key(),
            "enum.order_status.pending"
        );
        assert_eq!(
            OrderStatus::Reviewing.label_key(),
            "enum.order_status.reviewing"
        );
        assert_eq!(
            OrderStatus::Completed.label_key(),
            "enum.order_status.completed"
        );
    }

    #[test]
    fn string_backed_options() {
        let options = OrderStatus::options();
        assert_eq!(options.len(), 3);

        let opts: Vec<_> = options.into_iter().collect();
        assert_eq!(opts[0].value, EnumKey::String("pending".into()));
        assert_eq!(opts[0].label_key, "enum.order_status.pending");
        assert_eq!(opts[1].value, EnumKey::String("reviewing".into()));
        assert_eq!(opts[1].label_key, "enum.order_status.reviewing");
        assert_eq!(opts[2].value, EnumKey::String("completed".into()));
        assert_eq!(opts[2].label_key, "enum.order_status.completed");
    }

    #[test]
    fn string_backed_meta() {
        let meta = OrderStatus::meta();
        assert_eq!(meta.id, "order_status");
        assert_eq!(meta.key_kind, EnumKeyKind::String);
    }

    #[test]
    fn enum_level_label_prefix_changes_default_label_namespace() {
        assert_eq!(
            CreditTransactionType::AdminAdd.label_key(),
            "admin.credits.transaction_types.admin_add"
        );
        assert_eq!(
            CreditTransactionType::TransferSent.label_key(),
            "admin.credits.transaction_types.transfer_sent"
        );
    }

    // -----------------------------------------------------------------------
    // Override tests
    // -----------------------------------------------------------------------

    #[test]
    fn override_key() {
        assert_eq!(
            OrderStatusWithOverrides::Reviewing.key(),
            EnumKey::String("in_review".into())
        );
    }

    #[test]
    fn override_parse_key() {
        assert_eq!(
            OrderStatusWithOverrides::parse_key("in_review"),
            Some(OrderStatusWithOverrides::Reviewing)
        );
    }

    #[test]
    fn override_label_key() {
        assert_eq!(
            OrderStatusWithOverrides::Completed.label_key(),
            "Order completed"
        );
    }

    #[test]
    fn int_backed_label_key_uses_variant_name_not_numeric_value() {
        assert_eq!(UserStatus::Pending.label_key(), "enum.user_status.pending");
        assert_eq!(
            UserStatus::Verified.label_key(),
            "enum.user_status.verified"
        );
    }

    // -----------------------------------------------------------------------
    // Int-backed tests
    // -----------------------------------------------------------------------

    #[test]
    fn int_backed_key_returns_int() {
        assert_eq!(UserStatus::Pending.key(), EnumKey::Int(0));
    }

    #[test]
    fn int_backed_parse_key_string() {
        assert_eq!(UserStatus::parse_key("0"), Some(UserStatus::Pending));
        assert_eq!(UserStatus::parse_key("1"), Some(UserStatus::Verified));
        assert_eq!(UserStatus::parse_key("2"), Some(UserStatus::Suspended));
    }

    #[test]
    fn int_backed_parse_key_invalid() {
        assert_eq!(UserStatus::parse_key("99"), None);
    }

    #[test]
    fn int_backed_from_str_uses_parse_key() {
        assert_eq!("1".parse::<UserStatus>(), Ok(UserStatus::Verified));
        assert!("99".parse::<UserStatus>().is_err());
    }

    #[test]
    fn int_backed_key_kind() {
        assert_eq!(UserStatus::key_kind(), EnumKeyKind::Int);
    }

    #[test]
    fn int_backed_meta() {
        assert_eq!(UserStatus::meta().key_kind, EnumKeyKind::Int);
    }

    // -----------------------------------------------------------------------
    // Id tests
    // -----------------------------------------------------------------------

    #[test]
    fn id_inferred_from_type_name() {
        assert_eq!(OrderStatus::id(), "order_status");
    }

    #[test]
    fn id_explicit_override() {
        assert_eq!(CustomIdEnum::id(), "custom_status");
    }

    #[test]
    fn normalization_handles_digits_and_acronyms_for_keys() {
        assert_eq!(
            MixedIdentifierStatus::Credit1.key(),
            EnumKey::String("credit_1".into())
        );
        assert_eq!(
            MixedIdentifierStatus::HTTP2Enabled.key(),
            EnumKey::String("http_2_enabled".into())
        );
    }

    #[test]
    fn normalization_handles_digits_and_acronyms_for_label_keys() {
        assert_eq!(
            MixedIdentifierStatus::Credit1.label_key(),
            "enum.mixed_identifier_status.credit_1"
        );
        assert_eq!(
            MixedIdentifierStatus::HTTP2Enabled.label_key(),
            "enum.mixed_identifier_status.http_2_enabled"
        );
    }

    #[test]
    fn normalization_handles_digits_and_acronyms_for_enum_ids() {
        assert_eq!(HTTP2Setting::id(), "http_2_setting");
    }

    // -----------------------------------------------------------------------
    // DB_TYPE tests
    // -----------------------------------------------------------------------

    #[test]
    fn string_backed_db_type() {
        assert_eq!(OrderStatus::DB_TYPE, DbType::Text);
    }

    #[test]
    fn int_backed_db_type() {
        assert_eq!(UserStatus::DB_TYPE, DbType::Int32);
    }

    // -----------------------------------------------------------------------
    // ToDbValue tests
    // -----------------------------------------------------------------------

    #[test]
    fn to_db_value_string_backed() {
        assert_eq!(
            OrderStatus::Pending.to_db_value(),
            DbValue::Text("pending".into())
        );
    }

    #[test]
    fn to_db_value_int_backed() {
        assert_eq!(UserStatus::Verified.to_db_value(), DbValue::Int32(1));
    }

    #[test]
    fn to_db_value_exposes_db_type() {
        assert_eq!(<OrderStatus as ToDbValue>::db_type(), DbType::Text);
        assert_eq!(<UserStatus as ToDbValue>::db_type(), DbType::Int32);
    }

    // -----------------------------------------------------------------------
    // FromDbValue tests
    // -----------------------------------------------------------------------

    #[test]
    fn from_db_value_string_backed() {
        let result: crate::foundation::Result<OrderStatus> =
            FromDbValue::from_db_value(&DbValue::Text("pending".into()));
        assert_eq!(result.unwrap(), OrderStatus::Pending);
    }

    #[test]
    fn from_db_value_int_backed() {
        let result: crate::foundation::Result<UserStatus> =
            FromDbValue::from_db_value(&DbValue::Int32(1));
        assert_eq!(result.unwrap(), UserStatus::Verified);
    }

    #[test]
    fn from_db_value_invalid() {
        let result: std::result::Result<OrderStatus, _> =
            FromDbValue::from_db_value(&DbValue::Text("unknown".into()));
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Serde tests
    // -----------------------------------------------------------------------

    #[test]
    fn serde_string_backed_serialize() {
        let json = serde_json::to_string(&OrderStatus::Pending).unwrap();
        assert_eq!(json, "\"pending\"");
    }

    #[test]
    fn serde_string_backed_deserialize_valid() {
        let result: OrderStatus = serde_json::from_str("\"pending\"").unwrap();
        assert_eq!(result, OrderStatus::Pending);
    }

    #[test]
    fn serde_string_backed_deserialize_invalid() {
        let result = serde_json::from_str::<OrderStatus>("\"unknown\"");
        assert!(result.is_err());
    }

    #[test]
    fn serde_int_backed_serialize() {
        let json = serde_json::to_string(&UserStatus::Pending).unwrap();
        assert_eq!(json, "0");
    }

    #[test]
    fn serde_int_backed_deserialize_valid() {
        let result: UserStatus = serde_json::from_str("1").unwrap();
        assert_eq!(result, UserStatus::Verified);
    }

    #[test]
    fn serde_int_backed_deserialize_invalid() {
        let result = serde_json::from_str::<UserStatus>("99");
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Alias tests
    // -----------------------------------------------------------------------

    #[test]
    fn alias_parse_key() {
        assert_eq!(
            AliasedEnum::parse_key("awaiting"),
            Some(AliasedEnum::Pending)
        );
        assert_eq!(AliasedEnum::parse_key("queued"), Some(AliasedEnum::Pending));
    }

    #[test]
    fn alias_key_returns_canonical() {
        assert_eq!(
            AliasedEnum::Pending.key(),
            EnumKey::String("pending".into())
        );
    }

    #[test]
    fn alias_parse_canonical_still_works() {
        assert_eq!(
            AliasedEnum::parse_key("pending"),
            Some(AliasedEnum::Pending)
        );
    }

    #[test]
    fn aliases_are_exported_in_options_metadata() {
        let options = AliasedEnum::options();
        let opts: Vec<_> = options.into_iter().collect();

        assert_eq!(opts[0].value, EnumKey::String("pending".into()));
        assert_eq!(opts[0].aliases, vec!["awaiting", "queued"]);
        assert!(opts[1].aliases.is_empty());
    }

    #[test]
    fn accepted_keys_include_aliases() {
        assert_eq!(
            AliasedEnum::accepted_keys().into_vec(),
            vec!["pending", "awaiting", "queued", "active"]
        );
    }

    // -----------------------------------------------------------------------
    // Validation tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn validation_accepts_valid_key() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("status", "pending")
            .app_enum::<OrderStatus>()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn validation_accepts_alias_key() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("status", "awaiting")
            .app_enum::<AliasedEnum>()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn validation_rejects_invalid_key() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("status", "unknown")
            .app_enum::<OrderStatus>()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "app_enum");
    }
}
