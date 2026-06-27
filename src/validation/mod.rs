mod context;
mod executor;
mod extractor;
mod field;
pub mod file_rules;
mod from_multipart;
mod rules;
mod types;
mod validator;

pub use axum::extract::Multipart;
pub use context::{RuleContext, RuleRegistry, ValidationRule, ValidationRuleDescriptor};
pub use extractor::{JsonValidated, RequestValidator, Validated};
pub use field::{EachValidator, FieldValidator, KeyValidator};
pub use from_multipart::FromMultipart;
pub use types::{FieldError, ValidationError, ValidationErrorResponse, ValidationErrors};
pub use validator::Validator;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;

    use async_trait::async_trait;
    use tempfile::tempdir;

    use super::{RuleContext, RuleRegistry, ValidationError, ValidationRule, Validator};
    use crate::foundation::AppContext;
    use crate::support::ValidationRuleId;
    use crate::{config::ConfigRepository, foundation::Container};

    fn test_app() -> AppContext {
        AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap()
    }

    fn test_app_in_timezone(timezone: &str) -> AppContext {
        let directory = tempdir().unwrap();
        fs::write(
            directory.path().join("00-app.toml"),
            format!(
                r#"
                    [app]
                    timezone = "{timezone}"
                "#
            ),
        )
        .unwrap();

        AppContext::new(
            Container::new(),
            ConfigRepository::from_dir(directory.path()).unwrap(),
            RuleRegistry::new(),
        )
        .unwrap()
    }

    fn implemented_field_rule_count() -> usize {
        let rules_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/validation/rules.rs");
        let source = fs::read_to_string(rules_path).unwrap();
        let enum_start = source
            .find("enum FieldRule {")
            .expect("FieldRule enum should exist");
        let body_start = source[enum_start..]
            .find('{')
            .map(|offset| enum_start + offset + 1)
            .expect("FieldRule enum body should start");
        let body_end = source[body_start..]
            .find("\n#[derive(Clone)]\npub(crate) struct FieldStep")
            .map(|offset| body_start + offset)
            .expect("FieldRule enum body should end");

        source[body_start..body_end]
            .lines()
            .filter_map(|line| {
                let token = line
                    .trim_start()
                    .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                    .next()
                    .unwrap_or_default();
                token
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_uppercase())
                    .then_some(token)
            })
            .filter(|token| *token != "Named")
            .count()
    }

    #[test]
    fn validation_guide_rule_surface_count_tracks_field_rule_enum() {
        let count = implemented_field_rule_count();
        let guide_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/guides/validation.md");
        let guide = fs::read_to_string(guide_path).unwrap();
        let expected =
            format!("| **Total built-in field/collection validation features** | **{count}** |");

        assert_eq!(count, 89);
        assert!(
            guide.contains(&expected),
            "validation guide should publish the implementation-derived rule count `{expected}`"
        );

        for path in [
            "README.md",
            "docs/guides/README.md",
            "docs/api/index.md",
            "docs/api/modules/validation.md",
        ] {
            let full_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
            let content = fs::read_to_string(full_path).unwrap();
            assert!(
                content.contains("89 built-in field/collection validation features"),
                "{path} should mention the implementation-derived field/collection validation feature count"
            );
        }
    }

    struct MobileRule;

    struct PanickingRule;

    struct FactoryPanickingRule;

    #[async_trait]
    impl ValidationRule for MobileRule {
        async fn validate(
            &self,
            _context: &RuleContext,
            value: &str,
        ) -> std::result::Result<(), ValidationError> {
            if value.starts_with('+') && value[1..].chars().all(|ch| ch.is_ascii_digit()) {
                Ok(())
            } else {
                Err(ValidationError::new("mobile", "invalid mobile number"))
            }
        }
    }

    #[async_trait]
    impl ValidationRule for PanickingRule {
        async fn validate(
            &self,
            _context: &RuleContext,
            _value: &str,
        ) -> std::result::Result<(), ValidationError> {
            panic!("validation boom")
        }
    }

    impl ValidationRule for FactoryPanickingRule {
        fn validate<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            _context: &'life1 RuleContext,
            _value: &'life2 str,
        ) -> Pin<
            Box<
                dyn Future<Output = std::result::Result<(), ValidationError>> + Send + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            panic!("validation factory boom")
        }
    }

    #[tokio::test]
    async fn executes_custom_rules() {
        let rules = RuleRegistry::new();
        rules
            .register(ValidationRuleId::new("mobile"), MobileRule)
            .unwrap();
        let app = AppContext::new(Container::new(), ConfigRepository::empty(), rules).unwrap();
        let mut validator = Validator::new(app);

        validator
            .field("phone", "123")
            .required()
            .rule(ValidationRuleId::new("mobile"))
            .apply()
            .await
            .unwrap();

        let errors = validator.finish().unwrap_err();
        assert_eq!(errors.errors.len(), 1);
        assert_eq!(errors.errors[0].code, "mobile");
    }

    #[tokio::test]
    async fn custom_rule_panic_becomes_framework_error() {
        let rules = RuleRegistry::new();
        rules
            .register(ValidationRuleId::new("panic"), PanickingRule)
            .unwrap();
        let app = AppContext::new(Container::new(), ConfigRepository::empty(), rules).unwrap();
        let mut validator = Validator::new(app);

        let error = validator
            .field("phone", "123")
            .rule(ValidationRuleId::new("panic"))
            .apply()
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("validation rule `panic` panicked"));
        assert!(error.to_string().contains("validation boom"));
        assert!(validator.finish().is_ok());
    }

    #[tokio::test]
    async fn custom_rule_factory_panic_becomes_framework_error() {
        let rules = RuleRegistry::new();
        rules
            .register(ValidationRuleId::new("panic_factory"), FactoryPanickingRule)
            .unwrap();
        let app = AppContext::new(Container::new(), ConfigRepository::empty(), rules).unwrap();
        let mut validator = Validator::new(app);

        let error = validator
            .field("phone", "123")
            .rule(ValidationRuleId::new("panic_factory"))
            .apply()
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("validation rule `panic_factory` panicked"));
        assert!(error.to_string().contains("validation factory boom"));
        assert!(validator.finish().is_ok());
    }

    #[test]
    fn rejects_duplicate_named_rules() {
        let rules = RuleRegistry::new();
        rules
            .register(ValidationRuleId::new("mobile"), MobileRule)
            .unwrap();

        let error = rules
            .register(ValidationRuleId::new("mobile"), MobileRule)
            .unwrap_err();
        assert!(error.to_string().contains("already registered"));
    }

    #[test]
    fn descriptors_expose_registered_rule_ids_in_stable_order() {
        let rules = RuleRegistry::new();
        rules
            .register(ValidationRuleId::new("tenant.mobile"), MobileRule)
            .unwrap();
        rules
            .register(ValidationRuleId::new("account.slug"), MobileRule)
            .unwrap();

        let ids = rules
            .descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            vec![
                ValidationRuleId::new("account.slug"),
                ValidationRuleId::new("tenant.mobile"),
            ]
        );
    }

    // --- Email rule tests ---

    #[tokio::test]
    async fn email_rule_accepts_single_label_domain() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("email", "user@example")
            .email()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn email_rule_accepts_ip_literal_domain() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("email", "user@[127.0.0.1]")
            .email()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn email_rule_accepts_idn_domain() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("email", "test@domain.with.idn.tld.उदाहरण.परीक्षा")
            .email()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn email_rule_rejects_domain_underscore() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("email", "user@exa_mple.com")
            .email()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "email");
    }

    #[tokio::test]
    async fn email_rule_rejects_prefixed_ip_literal_domain() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("email", "user@prefix[127.0.0.1]")
            .email()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "email");
    }

    // --- Regex rule tests ---

    #[tokio::test]
    async fn regex_rule_accepts_matching_value() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("code", "ABC123")
            .regex(r"^[A-Z]{3}\d{3}$")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn regex_rule_accepts_unicode_character_classes() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("name", "Jos\u{e9}")
            .regex(r"^\p{L}+$")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn regex_rule_accepts_inline_case_insensitive_flags() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("slug", "Foundry")
            .regex(r"(?i)^foundry$")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn regex_rule_rejects_non_matching_value() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("code", "abc123")
            .regex(r"^[A-Z]{3}\d{3}$")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "regex");
    }

    #[tokio::test]
    async fn not_regex_rule_accepts_non_matching_value() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("username", "foundry_user")
            .not_regex(r"admin")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn not_regex_rule_rejects_matching_value() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("username", "admin_user")
            .not_regex(r"admin")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "username");
        assert_eq!(errors.errors[0].code, "not_regex");
        assert_eq!(
            errors.errors[0].message,
            "The username has an invalid format."
        );
    }

    // --- URL rule tests ---

    #[tokio::test]
    async fn url_rule_accepts_valid_urls() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("website", "https://example.com")
            .url()
            .apply()
            .await
            .unwrap();
        v.field("docs", "https://example.com/path%20with%20space")
            .url()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn url_rule_rejects_invalid_urls() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("website", "not-a-url").url().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "url");
    }

    #[tokio::test]
    async fn url_rule_rejects_whitespace_wrapped_urls() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("website", " https://example.com ")
            .url()
            .apply()
            .await
            .unwrap();
        v.field("docs", "https://example.com/path with space")
            .url()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert!(errors.errors.iter().all(|error| error.code == "url"));
    }

    #[tokio::test]
    async fn url_rule_rejects_empty_string() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("website", "").url().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "url");
    }

    // --- UUID rule tests ---

    #[tokio::test]
    async fn uuid_rule_accepts_valid_uuid() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("id", "550e8400-e29b-41d4-a716-446655440000")
            .uuid()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn uuid_rule_accepts_uuid_v7_and_common_parser_forms() {
        for value in [
            "01890f91-8e16-7cc2-bc8f-9f9c4d2f7f00",
            "01890f918e167cc2bc8f9f9c4d2f7f00",
            "{01890f91-8e16-7cc2-bc8f-9f9c4d2f7f00}",
            "urn:uuid:01890f91-8e16-7cc2-bc8f-9f9c4d2f7f00",
        ] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("id", value).uuid().apply().await.unwrap();
            assert!(v.finish().is_ok(), "value: {value:?}");
        }
    }

    #[tokio::test]
    async fn uuid_rule_rejects_invalid_uuid() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("id", "not-a-uuid").uuid().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "uuid");
    }

    #[tokio::test]
    async fn uuid_version_rule_accepts_only_requested_version() {
        let v4 = "550e8400-e29b-41d4-a716-446655440000";
        let v7 = "01890f91-8e16-7cc2-bc8f-9f9c4d2f7f00";

        let app = test_app();
        let mut validator = Validator::new(app);
        validator
            .field("id", v4)
            .uuid_version(4)
            .apply()
            .await
            .unwrap();
        assert!(validator.finish().is_ok());

        let app = test_app();
        let mut validator = Validator::new(app);
        validator
            .field("id", v7)
            .uuid_version(4)
            .apply()
            .await
            .unwrap();
        let errors = validator.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "uuid");
        assert_eq!(
            errors.errors[0].message,
            "The id must be a valid version 4 UUID."
        );
    }

    // --- ULID rule tests ---

    #[tokio::test]
    async fn ulid_rule_accepts_valid_ulids() {
        for value in ["01ARZ3NDEKTSV4RRFFQ69G5FAV", "01arz3ndektsv4rrffq69g5fav"] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("id", value).ulid().apply().await.unwrap();
            assert!(v.finish().is_ok(), "value: {value:?}");
        }
    }

    #[tokio::test]
    async fn ulid_rule_rejects_invalid_ulids() {
        for value in [
            "not-a-ulid",
            "81ARZ3NDEKTSV4RRFFQ69G5FAV",
            "01ARZ3NDEKTSV4RRFFQ69G5FAI",
        ] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("id", value).ulid().apply().await.unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].field, "id");
            assert_eq!(errors.errors[0].code, "ulid");
            assert_eq!(errors.errors[0].message, "The id must be a valid ULID.");
        }
    }

    // --- HexColor rule tests ---

    #[tokio::test]
    async fn hex_color_rule_accepts_hex_colors() {
        for value in ["#fff", "#ffff", "#00ff00", "#00FF0080"] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("color", value).hex_color().apply().await.unwrap();
            assert!(v.finish().is_ok(), "value: {value:?}");
        }
    }

    #[tokio::test]
    async fn hex_color_rule_rejects_invalid_hex_colors() {
        for value in ["fff", "#ff", "#fffff", "#ggg", "#00ff00ff00"] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("color", value).hex_color().apply().await.unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].field, "color");
            assert_eq!(errors.errors[0].code, "hex_color");
            assert_eq!(
                errors.errors[0].message,
                "The color must be a valid hexadecimal color."
            );
        }
    }

    // --- MacAddress rule tests ---

    #[tokio::test]
    async fn mac_address_rule_accepts_colon_and_hyphen_octets() {
        for value in [
            "00:1A:2B:3C:4D:5E",
            "aa:bb:cc:dd:ee:ff",
            "00-1a-2b-3c-4d-5e",
        ] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("device", value)
                .mac_address()
                .apply()
                .await
                .unwrap();
            assert!(v.finish().is_ok(), "value: {value:?}");
        }
    }

    #[tokio::test]
    async fn mac_address_rule_rejects_invalid_mac_addresses() {
        for value in [
            "001A2B3C4D5E",
            "00:1A:2B:3C:4D",
            "00:1A:2B:3C:4D:5E:6F",
            "00:1A:2B:3C:4D:ZZ",
            "00:1A-2B:3C:4D:5E",
        ] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("device", value)
                .mac_address()
                .apply()
                .await
                .unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].field, "device");
            assert_eq!(errors.errors[0].code, "mac_address");
            assert_eq!(
                errors.errors[0].message,
                "The device must be a valid MAC address."
            );
        }
    }

    #[tokio::test]
    async fn date_rule_accepts_valid_date() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("published_on", "2026-04-11")
            .date()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn date_rule_rejects_calendar_overflow() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("published_on", "2026-02-30")
            .date()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "date");
    }

    #[tokio::test]
    async fn time_rule_accepts_leap_second() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("starts_at", "13:15:60")
            .time()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn time_rule_requires_seconds() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("starts_at", "13:15").time().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "time");
    }

    #[tokio::test]
    async fn time_rule_rejects_invalid_time() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("starts_at", "25:00:00")
            .time()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "time");
    }

    #[tokio::test]
    async fn datetime_rule_uses_app_timezone_for_offset_less_values() {
        let app = test_app_in_timezone("Asia/Kuala_Lumpur");
        let mut v = Validator::new(app);
        v.field("published_at", "2026-04-11T13:00:00")
            .datetime()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn datetime_rule_accepts_rfc3339_space_separator() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("published_at", "2026-04-11 13:00:00Z")
            .datetime()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn datetime_rule_rejects_date_only() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("published_at", "2026-04-11")
            .datetime()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "datetime");
    }

    #[tokio::test]
    async fn datetime_rule_rejects_invalid_calendar_date() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("published_at", "2026-02-30T13:00:00")
            .datetime()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "datetime");
    }

    #[tokio::test]
    async fn local_datetime_rule_rejects_offset_aware_values() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("published_at", "2026-04-11T13:00:00+08:00")
            .local_datetime()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "local_datetime");
    }

    #[tokio::test]
    async fn local_datetime_rule_rejects_missing_seconds() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("published_at", "2026-04-11T13:00")
            .local_datetime()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "local_datetime");
    }

    #[tokio::test]
    async fn before_and_after_rules_compare_normalized_datetimes() {
        let app = test_app_in_timezone("Asia/Kuala_Lumpur");
        let mut v = Validator::new(app.clone());
        v.field("window_start", "2026-04-11T13:00:00")
            .before("window_end", "2026-04-11T14:00:00+08:00")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());

        let mut v = Validator::new(app);
        v.field("window_end", "2026-04-11T14:00:00")
            .after("window_start", "2026-04-11T14:00:00+08:00")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "after");
    }

    #[tokio::test]
    async fn date_equals_rule_compares_normalized_temporal_values() {
        let app = test_app_in_timezone("Asia/Kuala_Lumpur");
        let mut v = Validator::new(app.clone());
        v.field("scheduled_at", "2026-04-11T13:00:00")
            .date_equals("expected_at", "2026-04-11T13:00:00+08:00")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());

        let mut v = Validator::new(app);
        v.field("scheduled_at", "2026-04-11")
            .date_equals("expected_at", "2026-04-12")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "scheduled_at");
        assert_eq!(errors.errors[0].code, "date_equals");
        assert_eq!(
            errors.errors[0].message,
            "The scheduled_at must be a date equal to expected_at."
        );
    }

    // --- Numeric rule tests ---

    #[tokio::test]
    async fn numeric_rule_accepts_digits() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("amount", "123.45").numeric().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn numeric_rule_accepts_negative() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("amount", "-42").numeric().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn numeric_rule_rejects_letters() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("amount", "12abc").numeric().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "numeric");
    }

    // --- Decimal rule tests ---

    #[tokio::test]
    async fn decimal_rule_accepts_exact_decimal_places() {
        for value in ["12.30", "-0.99", "+.42"] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("amount", value)
                .decimal(2, 2)
                .apply()
                .await
                .unwrap();
            assert!(v.finish().is_ok(), "value: {value:?}");
        }
    }

    #[tokio::test]
    async fn decimal_rule_accepts_decimal_place_range() {
        for value in ["12.30", "12.345", "12.3456"] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("amount", value)
                .decimal(2, 4)
                .apply()
                .await
                .unwrap();
            assert!(v.finish().is_ok(), "value: {value:?}");
        }
    }

    #[tokio::test]
    async fn decimal_rule_rejects_invalid_precision_and_non_decimal_numbers() {
        for value in ["12", "12.3", "12.34567", "1e-2", "NaN", "abc"] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("amount", value)
                .decimal(2, 4)
                .apply()
                .await
                .unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].field, "amount");
            assert_eq!(errors.errors[0].code, "decimal");
            assert_eq!(
                errors.errors[0].message,
                "The amount must have between 2 and 4 decimal places."
            );
        }
    }

    #[tokio::test]
    async fn decimal_rule_uses_exact_decimal_message_when_min_equals_max() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("amount", "12.3")
            .decimal(2, 2)
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(
            errors.errors[0].message,
            "The amount must have 2 decimal places."
        );
    }

    // --- MultipleOf rule tests ---

    #[tokio::test]
    async fn multiple_of_rule_accepts_integer_and_decimal_multiples() {
        for (value, divisor) in [("15", 5.0), ("0.15", 0.05), ("-10", 2.5)] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("amount", value)
                .multiple_of(divisor)
                .apply()
                .await
                .unwrap();
            assert!(v.finish().is_ok(), "value: {value:?}, divisor: {divisor}");
        }
    }

    #[tokio::test]
    async fn multiple_of_rule_rejects_non_multiples_and_invalid_divisors() {
        for (value, divisor) in [
            ("14", 5.0),
            ("0.16", 0.05),
            ("abc", 5.0),
            ("15", 0.0),
            ("15", -5.0),
        ] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("amount", value)
                .multiple_of(divisor)
                .apply()
                .await
                .unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].field, "amount");
            assert_eq!(errors.errors[0].code, "multiple_of");
            assert_eq!(
                errors.errors[0].message,
                format!("The amount must be a multiple of {divisor}.")
            );
        }
    }

    // --- Alpha rule tests ---

    #[tokio::test]
    async fn alpha_rule_accepts_letters() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("name", "Jose\u{301}")
            .alpha()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn alpha_rule_accepts_empty_without_required() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("name", "").alpha().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn alpha_rule_rejects_digits() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("name", "Hello123").alpha().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "alpha");
    }

    // --- AlphaDash rule tests ---

    #[tokio::test]
    async fn alpha_dash_rule_accepts_unicode_letters_marks_numbers_dashes_and_underscores() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("username", "Jose\u{301}_user-123")
            .alpha_dash()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn alpha_dash_rule_accepts_empty_without_required() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("username", "").alpha_dash().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn alpha_dash_rule_rejects_other_punctuation() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("username", "user.name")
            .alpha_dash()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "username");
        assert_eq!(errors.errors[0].code, "alpha_dash");
        assert_eq!(
            errors.errors[0].message,
            "The username must contain only letters, numbers, dashes, and underscores."
        );
    }

    // --- AlphaNumeric rule tests ---

    #[tokio::test]
    async fn alpha_num_rule_accepts_laravel_alias() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("username", "Jose\u{301}123")
            .alpha_num()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn alpha_num_rule_rejects_special_chars() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("username", "user-123")
            .alpha_num()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "username");
        assert_eq!(errors.errors[0].code, "alpha_num");
    }

    #[tokio::test]
    async fn alpha_numeric_rule_accepts_letters_and_digits() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("username", "Jose\u{301}123")
            .alpha_numeric()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn alpha_numeric_rule_accepts_empty_without_required() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("username", "")
            .alpha_numeric()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn alpha_numeric_rule_rejects_special_chars() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("username", "user@123")
            .alpha_numeric()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "alpha_numeric");
    }

    // --- Ascii rule tests ---

    #[tokio::test]
    async fn ascii_rule_accepts_ascii_values() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("key", "foundry-api_123")
            .ascii()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn ascii_rule_accepts_empty_without_required() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("key", "").ascii().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn ascii_rule_rejects_non_ascii_values() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("key", "foundry-✓").ascii().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "key");
        assert_eq!(errors.errors[0].code, "ascii");
        assert_eq!(
            errors.errors[0].message,
            "The key must only contain ASCII characters."
        );
    }

    // --- Casing rule tests ---

    #[tokio::test]
    async fn lowercase_rule_accepts_lowercase_values() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("slug", "foundry-api")
            .lowercase()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn lowercase_rule_rejects_mixed_case_values() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("slug", "Foundry-API")
            .lowercase()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "slug");
        assert_eq!(errors.errors[0].code, "lowercase");
        assert_eq!(errors.errors[0].message, "The slug must be lowercase.");
    }

    #[tokio::test]
    async fn uppercase_rule_accepts_uppercase_values() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("code", "API_V2").uppercase().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn uppercase_rule_rejects_mixed_case_values() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("code", "Api_V2").uppercase().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "code");
        assert_eq!(errors.errors[0].code, "uppercase");
        assert_eq!(errors.errors[0].message, "The code must be uppercase.");
    }

    // --- InList rule tests ---

    #[tokio::test]
    async fn in_list_rule_accepts_valid_value() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("color", "red")
            .in_list(vec!["red", "green", "blue"])
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn in_list_rule_rejects_invalid_value() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("color", "yellow")
            .in_list(vec!["red", "green", "blue"])
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "in_list");
    }

    // --- NotIn rule tests ---

    #[tokio::test]
    async fn not_in_rule_accepts_valid_value() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("color", "yellow")
            .not_in(vec!["red", "green", "blue"])
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn not_in_rule_rejects_forbidden_value() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("color", "red")
            .not_in(vec!["red", "green", "blue"])
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "not_in");
    }

    // --- StartsWith rule tests ---

    #[tokio::test]
    async fn starts_with_rule_accepts_matching_prefix() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("path", "/api/users")
            .starts_with("/api")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn starts_with_rule_rejects_wrong_prefix() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("path", "/web/users")
            .starts_with("/api")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "starts_with");
    }

    #[tokio::test]
    async fn starts_with_any_rule_accepts_any_matching_prefix() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("path", "/admin/users")
            .starts_with_any(vec!["/api", "/admin"])
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn starts_with_any_rule_rejects_when_no_prefix_matches() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("path", "/web/users")
            .starts_with_any(vec!["/api", "/admin"])
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "starts_with");
        assert_eq!(
            errors.errors[0].message,
            "The path must start with /api, /admin."
        );
    }

    #[tokio::test]
    async fn doesnt_start_with_rule_accepts_different_prefix() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("path", "/web/users")
            .doesnt_start_with("/api")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn doesnt_start_with_rule_rejects_forbidden_prefix() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("path", "/api/users")
            .doesnt_start_with("/api")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "path");
        assert_eq!(errors.errors[0].code, "doesnt_start_with");
        assert_eq!(
            errors.errors[0].message,
            "The path must not start with /api."
        );
    }

    #[tokio::test]
    async fn doesnt_start_with_any_rule_rejects_any_forbidden_prefix() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("path", "/admin/users")
            .doesnt_start_with_any(vec!["/api", "/admin"])
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "path");
        assert_eq!(errors.errors[0].code, "doesnt_start_with");
        assert_eq!(
            errors.errors[0].message,
            "The path must not start with /api, /admin."
        );
    }

    // --- EndsWith rule tests ---

    #[tokio::test]
    async fn ends_with_rule_accepts_matching_suffix() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("file", "photo.png")
            .ends_with(".png")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn ends_with_rule_rejects_wrong_suffix() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("file", "photo.jpg")
            .ends_with(".png")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "ends_with");
    }

    #[tokio::test]
    async fn ends_with_any_rule_accepts_any_matching_suffix() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("file", "photo.jpg")
            .ends_with_any(vec![".png", ".jpg"])
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn ends_with_any_rule_rejects_when_no_suffix_matches() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("file", "photo.gif")
            .ends_with_any(vec![".png", ".jpg"])
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "ends_with");
        assert_eq!(
            errors.errors[0].message,
            "The file must end with .png, .jpg."
        );
    }

    #[tokio::test]
    async fn doesnt_end_with_rule_accepts_different_suffix() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("file", "photo.jpg")
            .doesnt_end_with(".png")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn doesnt_end_with_rule_rejects_forbidden_suffix() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("file", "photo.png")
            .doesnt_end_with(".png")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "file");
        assert_eq!(errors.errors[0].code, "doesnt_end_with");
        assert_eq!(errors.errors[0].message, "The file must not end with .png.");
    }

    #[tokio::test]
    async fn doesnt_end_with_any_rule_rejects_any_forbidden_suffix() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("file", "photo.jpg")
            .doesnt_end_with_any(vec![".png", ".jpg"])
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "file");
        assert_eq!(errors.errors[0].code, "doesnt_end_with");
        assert_eq!(
            errors.errors[0].message,
            "The file must not end with .png, .jpg."
        );
    }

    // --- Contains rule tests ---

    #[tokio::test]
    async fn contains_rule_accepts_matching_substring() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("scope", "admin:users:read")
            .contains(":users:")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn contains_rule_rejects_missing_substring() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("scope", "admin:roles:read")
            .contains(":users:")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "contains");
    }

    #[tokio::test]
    async fn doesnt_contain_rule_accepts_missing_substring() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("scope", "admin:users:read")
            .doesnt_contain(":root:")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn doesnt_contain_rule_rejects_matching_substring() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("scope", "admin:root:read")
            .doesnt_contain(":root:")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "scope");
        assert_eq!(errors.errors[0].code, "doesnt_contain");
        assert_eq!(
            errors.errors[0].message,
            "The scope must not contain :root:."
        );
    }

    // --- IP rule tests ---

    #[tokio::test]
    async fn ip_rule_accepts_valid_ipv4() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("addr", "192.168.1.1").ip().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn ip_rule_accepts_valid_ipv6() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("addr", "::1").ip().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn ip_rule_accepts_ipv4_mapped_ipv6() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("addr", "::ffff:192.0.2.128")
            .ip()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn ip_rule_rejects_invalid() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("addr", "999.999.999.999")
            .ip()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "ip");
    }

    #[tokio::test]
    async fn ip_rule_rejects_invalid_ipv6_shape() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("addr", "::::").ip().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "ip");
    }

    // --- JSON rule tests ---

    #[tokio::test]
    async fn json_rule_accepts_valid_json() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("data", r#"{"key":"value"}"#)
            .json()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn json_rule_rejects_invalid_json() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("data", "not json").json().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "json");
    }

    // --- Confirmed rule tests ---

    #[tokio::test]
    async fn confirmed_rule_accepts_matching_values() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("password", "secret123")
            .confirmed("password_confirmation", "secret123")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn confirmed_rule_rejects_mismatched_values() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("password", "secret123")
            .confirmed("password_confirmation", "different")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "confirmed");
    }

    // --- Digits rule tests ---

    #[tokio::test]
    async fn digits_rule_accepts_only_digits() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("zip", "12345").digits().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn digits_rule_accepts_empty_without_required() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("zip", "").digits().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn digits_rule_rejects_non_digits() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("zip", "12a45").digits().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "digits");
    }

    #[tokio::test]
    async fn digit_count_rules_accept_valid_lengths() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("pin_min", "1234")
            .min_digits(4)
            .apply()
            .await
            .unwrap();
        v.field("pin_max", "123456")
            .max_digits(6)
            .apply()
            .await
            .unwrap();
        v.field("pin_range", "12345")
            .digits_between(4, 6)
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn digit_count_rules_reject_invalid_lengths_and_non_digits() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("pin_min", "123")
            .min_digits(4)
            .apply()
            .await
            .unwrap();
        v.field("pin_max", "1234567")
            .max_digits(6)
            .apply()
            .await
            .unwrap();
        v.field("pin_range", "1234567")
            .digits_between(4, 6)
            .apply()
            .await
            .unwrap();
        v.field("pin_digits", "12a4")
            .digits_between(4, 6)
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "min_digits");
        assert_eq!(errors.errors[1].code, "max_digits");
        assert_eq!(errors.errors[2].code, "digits_between");
        assert_eq!(errors.errors[3].code, "digits_between");
    }

    // --- Timezone rule tests ---

    #[tokio::test]
    async fn timezone_rule_accepts_utc() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("tz", "UTC").timezone().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn timezone_rule_accepts_trimmed_case_insensitive_utc() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("tz", " utc ").timezone().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn timezone_rule_accepts_iana() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("tz", "Asia/Kuala_Lumpur")
            .timezone()
            .apply()
            .await
            .unwrap();
        v.field("legacy_tz", "US/Eastern")
            .timezone()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn timezone_rule_accepts_fixed_offset() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("tz", "+08:00").timezone().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn timezone_rule_accepts_compact_fixed_offset() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("tz", "+0800").timezone().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn timezone_rule_rejects_invalid() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("tz", "Invalid/Zone")
            .timezone()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "timezone");
    }

    #[tokio::test]
    async fn timezone_rule_rejects_browser_only_aliases() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("lowercase_tz", "asia/kuala_lumpur")
            .timezone()
            .apply()
            .await
            .unwrap();
        v.field("abbreviation", "PST")
            .timezone()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert!(errors.errors.iter().all(|error| error.code == "timezone"));
    }

    // --- Filled rule tests ---

    #[tokio::test]
    async fn filled_rule_accepts_non_empty_values() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("nickname", "Lin").filled().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn filled_rule_rejects_empty_and_whitespace_values() {
        for value in ["", "   "] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("nickname", value).filled().apply().await.unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].field, "nickname");
            assert_eq!(errors.errors[0].code, "filled");
            assert_eq!(
                errors.errors[0].message,
                "The nickname field must have a value."
            );
        }
    }

    #[tokio::test]
    async fn filled_collection_rule_rejects_empty_collections() {
        let app = test_app();
        let mut v = Validator::new(app);
        let tags: Vec<String> = Vec::new();
        v.each("tags", &tags)
            .filled_collection()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "tags");
        assert_eq!(errors.errors[0].code, "filled");
    }

    #[tokio::test]
    async fn each_filled_rule_rejects_empty_items() {
        let app = test_app();
        let mut v = Validator::new(app);
        let tags = vec!["rust".to_string(), "".to_string()];
        v.each("tags", &tags).filled().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "tags[1]");
        assert_eq!(errors.errors[0].code, "filled");
    }

    // --- Nullable tests ---

    #[tokio::test]
    async fn nullable_skips_rules_when_empty() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("nickname", "")
            .nullable()
            .email()
            .min(3)
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn nullable_validates_when_not_empty() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("nickname", "ab")
            .nullable()
            .email()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "email");
    }

    // --- Bail tests ---

    #[tokio::test]
    async fn bail_stops_on_first_error() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("name", "")
            .bail()
            .required()
            .email()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors.len(), 1); // Only required error, not email
    }

    // --- RequiredIf rule tests ---

    #[tokio::test]
    async fn required_if_rejects_empty_when_other_matches() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("publish_at", "")
            .required_if("status", "published", "published")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "publish_at");
        assert_eq!(errors.errors[0].code, "required_if");
        assert_eq!(
            errors.errors[0].message,
            "The publish_at field is required when status is published."
        );
    }

    #[tokio::test]
    async fn required_if_allows_empty_when_other_does_not_match() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("publish_at", "")
            .required_if("status", "draft", "published")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn required_unless_rejects_empty_when_other_does_not_match() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("publish_at", "")
            .required_unless("status", "published", "draft")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "publish_at");
        assert_eq!(errors.errors[0].code, "required_unless");
        assert_eq!(
            errors.errors[0].message,
            "The publish_at field is required unless status is draft."
        );
    }

    #[tokio::test]
    async fn required_unless_allows_empty_when_other_matches() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("publish_at", "")
            .required_unless("status", "draft", "draft")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn required_with_rejects_empty_when_other_present() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("phone_country", "")
            .required_with("phone_number", "+60123456789")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "phone_country");
        assert_eq!(errors.errors[0].code, "required_with");
        assert_eq!(
            errors.errors[0].message,
            "The phone_country field is required when phone_number is present."
        );
    }

    #[tokio::test]
    async fn required_with_allows_empty_when_other_empty() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("phone_country", "")
            .required_with("phone_number", "")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn required_with_all_rejects_empty_when_all_others_present() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("phone_label", "")
            .required_with_all(vec![
                ("phone_country", "MY"),
                ("phone_number", "+60123456789"),
            ])
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "phone_label");
        assert_eq!(errors.errors[0].code, "required_with_all");
        assert_eq!(
            errors.errors[0].message,
            "The phone_label field is required when phone_country, phone_number are present."
        );
    }

    #[tokio::test]
    async fn required_with_all_allows_empty_when_any_other_empty() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("phone_label", "")
            .required_with_all(vec![("phone_country", "MY"), ("phone_number", "")])
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn required_without_rejects_empty_when_other_empty() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("fallback_email", "")
            .required_without("phone_number", "")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "fallback_email");
        assert_eq!(errors.errors[0].code, "required_without");
        assert_eq!(
            errors.errors[0].message,
            "The fallback_email field is required when phone_number is not present."
        );
    }

    #[tokio::test]
    async fn required_without_allows_empty_when_other_present() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("fallback_email", "")
            .required_without("phone_number", "+60123456789")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn required_without_all_rejects_empty_when_all_others_empty() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("fallback_email", "")
            .required_without_all(vec![("phone_number", ""), ("phone_username", "")])
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "fallback_email");
        assert_eq!(errors.errors[0].code, "required_without_all");
        assert_eq!(
            errors.errors[0].message,
            "The fallback_email field is required when none of phone_number, phone_username are present."
        );
    }

    #[tokio::test]
    async fn required_without_all_allows_empty_when_any_other_present() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("fallback_email", "")
            .required_without_all(vec![
                ("phone_number", "+60123456789"),
                ("phone_username", ""),
            ])
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    // --- MinNumeric rule tests ---

    #[tokio::test]
    async fn min_numeric_accepts_above() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("age", "25").min_numeric(0.0).apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn min_numeric_rejects_below() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("age", "-1").min_numeric(0.0).apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "min_numeric");
    }

    #[tokio::test]
    async fn numeric_bounds_reject_unparseable_and_non_finite_input() {
        // Non-numeric, NaN, and infinity must not slip past bound checks:
        // NaN compares false against everything, and unparseable input
        // previously skipped the rule entirely.
        for value in ["abc", "NaN", "inf", "-inf", "1.2.3", "0x10", "0b10", ""] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("age", value)
                .min_numeric(0.0)
                .apply()
                .await
                .unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].code, "min_numeric", "value: {value:?}");

            let app = test_app();
            let mut v = Validator::new(app);
            v.field("age", value)
                .max_numeric(10.0)
                .apply()
                .await
                .unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].code, "max_numeric", "value: {value:?}");

            let app = test_app();
            let mut v = Validator::new(app);
            v.field("age", value)
                .between(0.0, 10.0)
                .apply()
                .await
                .unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].code, "between", "value: {value:?}");
        }
    }

    // --- MaxNumeric rule tests ---

    #[tokio::test]
    async fn max_numeric_rejects_above() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("age", "200")
            .max_numeric(150.0)
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "max_numeric");
    }

    // --- Integer rule tests ---

    #[tokio::test]
    async fn integer_accepts_valid() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("count", "42").integer().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn integer_accepts_plus_sign_and_i64_bounds() {
        for value in ["+42", "9223372036854775807", "-9223372036854775808"] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("count", value).integer().apply().await.unwrap();
            assert!(v.finish().is_ok(), "value: {value:?}");
        }
    }

    #[tokio::test]
    async fn integer_rejects_decimal() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("count", "3.14").integer().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "integer");
    }

    #[tokio::test]
    async fn integer_rejects_whitespace_and_i64_overflow() {
        for value in [" 42", "42 ", "9223372036854775808", "-9223372036854775809"] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("count", value).integer().apply().await.unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].code, "integer", "value: {value:?}");
        }
    }

    // --- Boolean rule tests ---

    #[tokio::test]
    async fn boolean_accepts_bool_like_values() {
        for value in ["true", "false", "1", "0", " true "] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("enabled", value).boolean().apply().await.unwrap();
            assert!(v.finish().is_ok(), "value: {value:?}");
        }
    }

    #[tokio::test]
    async fn boolean_rejects_non_bool_like_values() {
        for value in ["yes", "on", "2", ""] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("enabled", value).boolean().apply().await.unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].code, "boolean", "value: {value:?}");
        }
    }

    // --- Accepted / declined rule tests ---

    #[tokio::test]
    async fn accepted_accepts_laravel_checkbox_values() {
        for value in ["yes", "on", "1", "true", " yes "] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("terms", value).accepted().apply().await.unwrap();
            assert!(v.finish().is_ok(), "value: {value:?}");
        }
    }

    #[tokio::test]
    async fn accepted_rejects_declined_and_empty_values() {
        for value in ["no", "off", "0", "false", ""] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("terms", value).accepted().apply().await.unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].code, "accepted", "value: {value:?}");
        }
    }

    #[tokio::test]
    async fn accepted_if_rejects_when_other_matches() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("terms", "no")
            .accepted_if("requires_terms", "true", "true")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "terms");
        assert_eq!(errors.errors[0].code, "accepted_if");
        assert_eq!(
            errors.errors[0].message,
            "The terms must be accepted when requires_terms is true."
        );
    }

    #[tokio::test]
    async fn accepted_if_allows_any_value_when_other_does_not_match() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("terms", "")
            .accepted_if("requires_terms", "false", "true")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn declined_accepts_laravel_checkbox_values() {
        for value in ["no", "off", "0", "false", " off "] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("terms", value).declined().apply().await.unwrap();
            assert!(v.finish().is_ok(), "value: {value:?}");
        }
    }

    #[tokio::test]
    async fn declined_rejects_accepted_and_empty_values() {
        for value in ["yes", "on", "1", "true", ""] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("terms", value).declined().apply().await.unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].code, "declined", "value: {value:?}");
        }
    }

    #[tokio::test]
    async fn declined_if_rejects_when_other_matches() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("marketing_opt_out", "yes")
            .declined_if("requires_opt_out", "true", "true")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "marketing_opt_out");
        assert_eq!(errors.errors[0].code, "declined_if");
        assert_eq!(
            errors.errors[0].message,
            "The marketing_opt_out must be declined when requires_opt_out is true."
        );
    }

    #[tokio::test]
    async fn declined_if_allows_any_value_when_other_does_not_match() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("marketing_opt_out", "")
            .declined_if("requires_opt_out", "false", "true")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn required_if_accepted_rejects_empty_when_other_is_accepted() {
        for accepted in ["yes", "on", "1", "true"] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("terms_note", "")
                .required_if_accepted("terms", accepted)
                .apply()
                .await
                .unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].field, "terms_note");
            assert_eq!(errors.errors[0].code, "required_if_accepted");
            assert_eq!(
                errors.errors[0].message,
                "The terms_note field is required when terms is accepted."
            );
        }
    }

    #[tokio::test]
    async fn required_if_accepted_allows_empty_when_other_is_not_accepted() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("terms_note", "")
            .required_if_accepted("terms", "no")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn required_if_declined_rejects_empty_when_other_is_declined() {
        for declined in ["no", "off", "0", "false"] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("marketing_reason", "")
                .required_if_declined("marketing_opt_out", declined)
                .apply()
                .await
                .unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].field, "marketing_reason");
            assert_eq!(errors.errors[0].code, "required_if_declined");
            assert_eq!(
                errors.errors[0].message,
                "The marketing_reason field is required when marketing_opt_out is declined."
            );
        }
    }

    #[tokio::test]
    async fn required_if_declined_allows_empty_when_other_is_not_declined() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("marketing_reason", "")
            .required_if_declined("marketing_opt_out", "yes")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn prohibited_rejects_non_empty_values() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("legacy_token", "secret")
            .prohibited()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "legacy_token");
        assert_eq!(errors.errors[0].code, "prohibited");
        assert_eq!(
            errors.errors[0].message,
            "The legacy_token field is prohibited."
        );
    }

    #[tokio::test]
    async fn prohibited_allows_empty_values() {
        for value in ["", "   "] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("legacy_token", value)
                .prohibited()
                .apply()
                .await
                .unwrap();
            assert!(v.finish().is_ok(), "value: {value:?}");
        }
    }

    #[tokio::test]
    async fn prohibited_if_rejects_when_other_matches() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("remember_override", "force")
            .prohibited_if("remember", "true", "true")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "remember_override");
        assert_eq!(errors.errors[0].code, "prohibited_if");
        assert_eq!(
            errors.errors[0].message,
            "The remember_override field is prohibited when remember is true."
        );
    }

    #[tokio::test]
    async fn prohibited_if_allows_non_empty_when_other_does_not_match() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("remember_override", "force")
            .prohibited_if("remember", "false", "true")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn prohibited_unless_rejects_when_other_does_not_match() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("guest_override", "force")
            .prohibited_unless("remember", "false", "true")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "guest_override");
        assert_eq!(errors.errors[0].code, "prohibited_unless");
        assert_eq!(
            errors.errors[0].message,
            "The guest_override field is prohibited unless remember is true."
        );
    }

    #[tokio::test]
    async fn prohibited_unless_allows_non_empty_when_other_matches() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("guest_override", "force")
            .prohibited_unless("remember", "true", "true")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn prohibited_if_accepted_rejects_non_empty_when_other_is_accepted() {
        for accepted in ["yes", "on", "1", "true"] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("terms_override", "force")
                .prohibited_if_accepted("terms", accepted)
                .apply()
                .await
                .unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].field, "terms_override");
            assert_eq!(errors.errors[0].code, "prohibited_if_accepted");
            assert_eq!(
                errors.errors[0].message,
                "The terms_override field is prohibited when terms is accepted."
            );
        }
    }

    #[tokio::test]
    async fn prohibited_if_declined_rejects_non_empty_when_other_is_declined() {
        for declined in ["no", "off", "0", "false"] {
            let app = test_app();
            let mut v = Validator::new(app);
            v.field("marketing_override", "force")
                .prohibited_if_declined("marketing_opt_out", declined)
                .apply()
                .await
                .unwrap();
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].field, "marketing_override");
            assert_eq!(errors.errors[0].code, "prohibited_if_declined");
            assert_eq!(
                errors.errors[0].message,
                "The marketing_override field is prohibited when marketing_opt_out is declined."
            );
        }
    }

    #[tokio::test]
    async fn prohibits_rejects_present_fields_when_value_is_present() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("exclusive_email", "primary@example.com")
            .prohibits(vec![("phone", "+60123456789"), ("backup_email", "")])
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "exclusive_email");
        assert_eq!(errors.errors[0].code, "prohibits");
        assert_eq!(
            errors.errors[0].message,
            "The exclusive_email field prohibits phone."
        );
    }

    #[tokio::test]
    async fn prohibits_allows_empty_value_or_empty_fields() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("exclusive_email", "")
            .prohibits(vec![("phone", "+60123456789")])
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());

        let app = test_app();
        let mut v = Validator::new(app);
        v.field("exclusive_email", "primary@example.com")
            .prohibits(vec![("phone", ""), ("backup_email", "   ")])
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    // --- Between rule tests ---

    #[tokio::test]
    async fn between_accepts_in_range() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("score", "85")
            .between(0.0, 100.0)
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn between_rejects_out_of_range() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("score", "150")
            .between(0.0, 100.0)
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "between");
    }

    #[tokio::test]
    async fn numeric_comparison_rules_accept_strict_and_inclusive_bounds() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("discount", "5.1").gt(5.0).apply().await.unwrap();
        v.field("minimum", "5").gte(5.0).apply().await.unwrap();
        v.field("ceiling", "9.9").lt(10.0).apply().await.unwrap();
        v.field("maximum", "10").lte(10.0).apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn numeric_comparison_rules_reject_failed_bounds_and_invalid_numbers() {
        let cases = [
            ("discount", "5", "gt"),
            ("minimum", "4.99", "gte"),
            ("ceiling", "10", "lt"),
            ("maximum", "10.01", "lte"),
            ("invalid", "NaN", "gt"),
        ];

        for (field, value, expected_code) in cases {
            let app = test_app();
            let mut v = Validator::new(app);
            match expected_code {
                "gt" => v.field(field, value).gt(5.0).apply().await.unwrap(),
                "gte" => v.field(field, value).gte(5.0).apply().await.unwrap(),
                "lt" => v.field(field, value).lt(10.0).apply().await.unwrap(),
                "lte" => v.field(field, value).lte(10.0).apply().await.unwrap(),
                _ => unreachable!(),
            }
            let errors = v.finish().unwrap_err();
            assert_eq!(errors.errors[0].field, field);
            assert_eq!(errors.errors[0].code, expected_code);
        }
    }

    #[tokio::test]
    async fn size_rule_checks_exact_character_length() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("code", "RUST").size(4).apply().await.unwrap();
        assert!(v.finish().is_ok());

        let app = test_app();
        let mut v = Validator::new(app);
        v.field("code", "Ferris").size(4).apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "code");
        assert_eq!(errors.errors[0].code, "size");
        assert_eq!(errors.errors[0].message, "The code must be exactly 4.");
    }

    #[tokio::test]
    async fn min_length_and_max_length_aliases_use_canonical_codes() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("short_code", "rs")
            .min_length(3)
            .apply()
            .await
            .unwrap();
        v.field("long_code", "rustacean")
            .max_length(4)
            .apply()
            .await
            .unwrap();

        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors.len(), 2);
        assert_eq!(errors.errors[0].field, "short_code");
        assert_eq!(errors.errors[0].code, "min");
        assert_eq!(errors.errors[1].field, "long_code");
        assert_eq!(errors.errors[1].code, "max");
    }

    #[tokio::test]
    async fn size_numeric_rule_checks_exact_number() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("seats", "10.0")
            .size_numeric(10.0)
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());

        let app = test_app();
        let mut v = Validator::new(app);
        v.field("seats", "9")
            .size_numeric(10.0)
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "seats");
        assert_eq!(errors.errors[0].code, "size");
    }

    // --- Ipv4 rule tests ---

    #[tokio::test]
    async fn ipv4_accepts_valid() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("ip", "192.168.1.1").ipv4().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn ipv4_rejects_leading_zero_octets() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("ip", "01.02.03.04").ipv4().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "ipv4");
    }

    #[tokio::test]
    async fn ipv4_rejects_ipv6() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("ip", "::1").ipv4().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "ipv4");
    }

    // --- Ipv6 rule tests ---

    #[tokio::test]
    async fn ipv6_accepts_valid() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("ip", "::1").ipv6().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn ipv6_accepts_ipv4_mapped_address() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("ip", "::ffff:192.0.2.128")
            .ipv6()
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn ipv6_rejects_ipv4() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("ip", "192.168.1.1").ipv6().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "ipv6");
    }

    #[tokio::test]
    async fn ipv6_rejects_invalid_shape() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("ip", "::::").ipv6().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "ipv6");
    }

    // --- Same rule tests ---

    #[tokio::test]
    async fn same_accepts_matching() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("password", "secret123")
            .same("password_confirmation", "secret123")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn same_rejects_mismatch() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("password", "secret123")
            .same("password_confirmation", "different")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "same");
    }

    // --- Different rule tests ---

    #[tokio::test]
    async fn different_accepts_different() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("new_email", "new@test.com")
            .different("current_email", "old@test.com")
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn different_rejects_same() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("new_email", "same@test.com")
            .different("current_email", "same@test.com")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "different");
    }

    // --- Required keys rule tests ---

    #[tokio::test]
    async fn required_keys_accepts_present_keys() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.keys("settings", ["timezone", "locale", "theme"])
            .required_keys(["timezone", "locale"])
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn required_keys_rejects_missing_keys() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.keys("settings", ["timezone"])
            .required_keys(["timezone", "locale", "theme"])
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "settings");
        assert_eq!(errors.errors[0].code, "required_keys");
        assert_eq!(
            errors.errors[0].message,
            "The settings field must contain entries for locale, theme."
        );
    }

    #[tokio::test]
    async fn required_keys_supports_inline_message() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.keys("settings", ["timezone"])
            .required_keys(["timezone", "locale"])
            .with_message("Missing required settings: {{keys}}")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(
            errors.errors[0].message,
            "Missing required settings: locale"
        );
    }

    // --- Unique rule tests ---

    #[tokio::test]
    async fn unique_adds_rule_to_steps() {
        let app = test_app();
        let mut v = Validator::new(app);
        // The apply will fail because no database is configured in test_app
        v.field("email", "test@example.com")
            .unique("users", "email")
            .apply()
            .await
            .unwrap_err();
    }

    // --- Exists rule tests ---

    #[tokio::test]
    async fn exists_adds_rule_to_steps() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("country_id", "1")
            .exists("countries", "id")
            .apply()
            .await
            .unwrap_err();
    }

    // --- Translation-aware validation tests ---

    fn test_app_with_i18n() -> (AppContext, tempfile::TempDir) {
        use crate::config::I18nConfig;
        use crate::i18n::I18nManager;
        use std::sync::Arc;

        let dir = tempdir().unwrap();
        let locale_dir = dir.path().join("en");
        fs::create_dir_all(&locale_dir).unwrap();
        fs::write(
            locale_dir.join("validation.json"),
            r#"{
                "validation": {
                    "required": "The {{attribute}} field is required.",
                    "email": "The {{attribute}} must be a valid email address.",
                    "min": "The {{attribute}} must be at least {{min}} characters.",
                    "custom": {
                        "tags": {
                            "required": "Every {{attribute}} is required."
                        }
                    },
                    "attributes": {
                        "email": "email address",
                        "tags": "tag"
                    }
                }
            }"#,
        )
        .unwrap();
        fs::write(
            locale_dir.join("admin.json"),
            r#"{
                "admin": {
                    "credits": {
                        "fields": {
                            "amount": "Amount"
                        }
                    }
                }
            }"#,
        )
        .unwrap();

        let ms_dir = dir.path().join("ms");
        fs::create_dir_all(&ms_dir).unwrap();
        fs::write(
            ms_dir.join("validation.json"),
            r#"{
                "validation": {
                    "required": "Medan {{attribute}} adalah wajib.",
                    "attributes": {
                        "email": "alamat e-mel"
                    }
                }
            }"#,
        )
        .unwrap();
        fs::write(
            ms_dir.join("admin.json"),
            r#"{
                "admin": {
                    "credits": {
                        "fields": {
                            "amount": "Jumlah"
                        }
                    }
                }
            }"#,
        )
        .unwrap();

        let config = I18nConfig {
            default_locale: "en".to_string(),
            fallback_locale: "en".to_string(),
            resource_path: dir.path().to_str().unwrap().to_string(),
        };
        let manager = I18nManager::load(&config).unwrap();

        let container = Container::new();
        container.singleton_arc(Arc::new(manager)).unwrap();

        let app =
            AppContext::new(container, ConfigRepository::empty(), RuleRegistry::new()).unwrap();
        (app, dir)
    }

    #[tokio::test]
    async fn default_messages_use_fallback_when_no_i18n() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("email", "").required().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "required");
        assert_eq!(errors.errors[0].message, "The email field is required.");
    }

    #[tokio::test]
    async fn translates_messages_from_i18n() {
        let (app, _dir) = test_app_with_i18n();
        let mut v = Validator::new(app);
        v.field("email", "").required().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(
            errors.errors[0].message,
            "The email address field is required."
        );
    }

    #[tokio::test]
    async fn translates_messages_with_locale() {
        let (app, _dir) = test_app_with_i18n();
        let mut v = Validator::new(app).locale("ms");
        v.field("email", "").required().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].message, "Medan alamat e-mel adalah wajib.");
    }

    #[tokio::test]
    async fn translates_messages_with_params() {
        let (app, _dir) = test_app_with_i18n();
        let mut v = Validator::new(app);
        v.field("password", "ab").min(8).apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(
            errors.errors[0].message,
            "The password must be at least 8 characters."
        );
    }

    #[tokio::test]
    async fn with_message_overrides_translation() {
        let (app, _dir) = test_app_with_i18n();
        let mut v = Validator::new(app);
        v.field("email", "")
            .required()
            .with_message("We need your email!")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].message, "We need your email!");
    }

    #[tokio::test]
    async fn with_message_supports_placeholders() {
        let (app, _dir) = test_app_with_i18n();
        let mut v = Validator::new(app);
        v.field("email", "")
            .required()
            .with_message("Please provide the {{attribute}}.")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(
            errors.errors[0].message,
            "Please provide the email address."
        );
    }

    #[tokio::test]
    async fn custom_message_overrides_translation() {
        let (app, _dir) = test_app_with_i18n();
        let mut v = Validator::new(app);
        v.custom_message("email", "required", "Email is mandatory!");
        v.field("email", "").required().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].message, "Email is mandatory!");
    }

    #[tokio::test]
    async fn custom_attribute_overrides_field_name() {
        let (app, _dir) = test_app_with_i18n();
        let mut v = Validator::new(app);
        v.custom_attribute("email", "work email");
        v.field("email", "").required().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(
            errors.errors[0].message,
            "The work email field is required."
        );
    }

    #[tokio::test]
    async fn custom_attribute_can_use_translation_key() {
        let (app, _dir) = test_app_with_i18n();
        let mut v = Validator::new(app).locale("ms");
        v.custom_attribute("amount", "admin.credits.fields.amount");
        v.field("amount", "").required().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].message, "Medan Jumlah adalah wajib.");
    }

    #[tokio::test]
    async fn with_message_has_highest_priority() {
        let (app, _dir) = test_app_with_i18n();
        let mut v = Validator::new(app);
        v.custom_message("email", "required", "Custom from validator");
        v.field("email", "")
            .required()
            .with_message("Inline message wins")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].message, "Inline message wins");
    }

    #[tokio::test]
    async fn named_rule_with_with_message() {
        let rules = RuleRegistry::new();
        rules
            .register(ValidationRuleId::new("mobile"), MobileRule)
            .unwrap();
        let app = AppContext::new(Container::new(), ConfigRepository::empty(), rules).unwrap();
        let mut v = Validator::new(app);
        v.field("phone", "123")
            .rule(ValidationRuleId::new("mobile"))
            .with_message("Invalid phone format")
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "mobile");
        assert_eq!(errors.errors[0].message, "Invalid phone format");
    }

    #[tokio::test]
    async fn named_rule_without_with_message_uses_resolved_message() {
        let rules = RuleRegistry::new();
        rules
            .register(ValidationRuleId::new("mobile"), MobileRule)
            .unwrap();
        let app = AppContext::new(Container::new(), ConfigRepository::empty(), rules).unwrap();
        let mut v = Validator::new(app);
        v.field("phone", "123")
            .rule(ValidationRuleId::new("mobile"))
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "mobile");
        // Without i18n configured, resolve_message falls back to generic message.
        // With i18n, it would resolve validation.mobile from the locale file.
        assert_eq!(errors.errors[0].message, "The phone is invalid.");
    }

    // --- EachValidator tests ---

    #[tokio::test]
    async fn each_validates_all_items() {
        let app = test_app();
        let items = vec!["ab".to_string(), "".to_string(), "c".to_string()];
        let mut v = Validator::new(app);
        v.each("tags", &items)
            .required()
            .min(2)
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors.len(), 2);
        assert_eq!(errors.errors[0].field, "tags[1]");
        assert_eq!(errors.errors[0].code, "required");
        assert_eq!(errors.errors[1].field, "tags[2]");
        assert_eq!(errors.errors[1].code, "min");
    }

    #[tokio::test]
    async fn each_with_no_errors() {
        let app = test_app();
        let items = vec!["rust".to_string(), "foundry".to_string()];
        let mut v = Validator::new(app);
        v.each("tags", &items)
            .required()
            .min(2)
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn each_min_items_accepts_enough_items() {
        let app = test_app();
        let items = vec!["rust".to_string(), "foundry".to_string()];
        let mut v = Validator::new(app);
        v.each("tags", &items).min_items(2).apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn each_min_items_rejects_too_few_items() {
        let app = test_app();
        let items = vec!["rust".to_string()];
        let mut v = Validator::new(app);
        v.each("tags", &items).min_items(2).apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "tags");
        assert_eq!(errors.errors[0].code, "min_items");
        assert_eq!(
            errors.errors[0].message,
            "The tags must contain at least 2 items."
        );
    }

    #[tokio::test]
    async fn each_max_items_rejects_too_many_items() {
        let app = test_app();
        let items = vec![
            "rust".to_string(),
            "foundry".to_string(),
            "typed".to_string(),
        ];
        let mut v = Validator::new(app);
        v.each("tags", &items).max_items(2).apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "tags");
        assert_eq!(errors.errors[0].code, "max_items");
        assert_eq!(
            errors.errors[0].message,
            "The tags must not contain more than 2 items."
        );
    }

    #[tokio::test]
    async fn each_size_items_checks_exact_count() {
        let app = test_app();
        let items = vec!["rust".to_string(), "foundry".to_string()];
        let mut v = Validator::new(app);
        v.each("tags", &items).size_items(2).apply().await.unwrap();
        assert!(v.finish().is_ok());

        let app = test_app();
        let items = vec!["rust".to_string()];
        let mut v = Validator::new(app);
        v.each("tags", &items).size_items(2).apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "tags");
        assert_eq!(errors.errors[0].code, "size");
        assert_eq!(errors.errors[0].message, "The tags must be exactly 2.");
    }

    #[tokio::test]
    async fn each_distinct_accepts_unique_items() {
        let app = test_app();
        let items = vec!["rust".to_string(), "foundry".to_string()];
        let mut v = Validator::new(app);
        v.each("tags", &items).distinct().apply().await.unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn each_distinct_rejects_duplicate_items() {
        let app = test_app();
        let items = vec![
            "rust".to_string(),
            "foundry".to_string(),
            "rust".to_string(),
        ];
        let mut v = Validator::new(app);
        v.each("tags", &items).distinct().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "tags");
        assert_eq!(errors.errors[0].code, "distinct");
        assert_eq!(
            errors.errors[0].message,
            "The tags must not contain duplicate items."
        );
    }

    #[tokio::test]
    async fn each_contains_all_accepts_required_values() {
        let app = test_app();
        let items = vec![
            "admin".to_string(),
            "editor".to_string(),
            "viewer".to_string(),
        ];
        let mut v = Validator::new(app);
        v.each("roles", &items)
            .contains_all(vec!["admin", "editor"])
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn each_contains_all_rejects_missing_required_values() {
        let app = test_app();
        let items = vec!["editor".to_string(), "viewer".to_string()];
        let mut v = Validator::new(app);
        v.each("roles", &items)
            .contains_all(vec!["admin", "editor"])
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "roles");
        assert_eq!(errors.errors[0].code, "contains");
        assert_eq!(
            errors.errors[0].message,
            "The roles must contain admin, editor."
        );
    }

    #[tokio::test]
    async fn each_doesnt_contain_any_rejects_forbidden_values() {
        let app = test_app();
        let items = vec!["admin".to_string(), "editor".to_string()];
        let mut v = Validator::new(app);
        v.each("roles", &items)
            .doesnt_contain_any(vec!["admin", "root"])
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "roles");
        assert_eq!(errors.errors[0].code, "doesnt_contain");
        assert_eq!(
            errors.errors[0].message,
            "The roles must not contain admin, root."
        );
    }

    #[tokio::test]
    async fn each_bail_stops_item_rules_after_collection_error() {
        let app = test_app();
        let items = vec!["".to_string()];
        let mut v = Validator::new(app);
        v.each("tags", &items)
            .bail()
            .min_items(2)
            .required()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors.len(), 1);
        assert_eq!(errors.errors[0].field, "tags");
        assert_eq!(errors.errors[0].code, "min_items");
    }

    #[tokio::test]
    async fn each_nullable_skips_empty() {
        let app = test_app();
        let items = vec!["rust".to_string(), "".to_string(), "foundry".to_string()];
        let mut v = Validator::new(app);
        v.each("tags", &items)
            .nullable()
            .min(2)
            .apply()
            .await
            .unwrap();
        assert!(v.finish().is_ok());
    }

    #[tokio::test]
    async fn each_with_custom_attribute() {
        let app = test_app();
        let items = vec!["".to_string()];
        let mut v = Validator::new(app);
        v.custom_attribute("tags", "tag");
        v.each("tags", &items).required().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "tags[0]");
        assert_eq!(errors.errors[0].message, "The tag field is required.");
    }

    #[tokio::test]
    async fn each_with_custom_message_uses_base_field() {
        let app = test_app();
        let items = vec!["".to_string()];
        let mut v = Validator::new(app);
        v.custom_attribute("tags", "tag");
        v.custom_message("tags", "required", "Every {{attribute}} is required.");
        v.each("tags", &items).required().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "tags[0]");
        assert_eq!(errors.errors[0].message, "Every tag is required.");
    }

    #[tokio::test]
    async fn indexed_nested_fields_use_unindexed_base_message_metadata() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.custom_attribute("addresses.street_name", "street name");
        v.custom_message(
            "addresses.street_name",
            "required",
            "Every {{attribute}} is required.",
        );
        v.field("addresses[0].street_name", "")
            .required()
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "addresses[0].street_name");
        assert_eq!(errors.errors[0].message, "Every street name is required.");
    }

    #[tokio::test]
    async fn each_with_i18n_custom_message_uses_base_field() {
        let (app, _dir) = test_app_with_i18n();
        let items = vec!["".to_string()];
        let mut v = Validator::new(app);
        v.each("tags", &items).required().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].field, "tags[0]");
        assert_eq!(errors.errors[0].message, "Every tag is required.");
    }

    #[tokio::test]
    async fn each_with_bail() {
        let app = test_app();
        let items = vec!["".to_string()];
        let mut v = Validator::new(app);
        v.each("tags", &items)
            .bail()
            .required()
            .min(2)
            .apply()
            .await
            .unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors.len(), 1);
        assert_eq!(errors.errors[0].code, "required");
    }
}
