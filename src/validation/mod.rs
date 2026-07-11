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
pub use context::{RuleContext, RuleRegistry, ValidationRule};
pub use extractor::{JsonValidated, RequestValidator, Validated};
pub use field::{EachValidator, FieldValidator};
pub use from_multipart::FromMultipart;
pub use types::{FieldError, ValidationError, ValidationErrors};
pub use validator::Validator;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::future::Future;
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

    struct MobileRule;

    struct PanickingRule;

    struct FactoryPanickingRule;

    struct PlaceholderRule;

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
    impl ValidationRule for PlaceholderRule {
        async fn validate(
            &self,
            _context: &RuleContext,
            _value: &str,
        ) -> std::result::Result<(), ValidationError> {
            Err(ValidationError::new(
                "placeholder",
                "The {{attribute}} failed its custom rule.",
            ))
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
    async fn uuid_rule_rejects_invalid_uuid() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("id", "not-a-uuid").uuid().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "uuid");
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

    // --- Alpha rule tests ---

    #[tokio::test]
    async fn alpha_rule_accepts_letters() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("name", "Hello").alpha().apply().await.unwrap();
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

    // --- AlphaNumeric rule tests ---

    #[tokio::test]
    async fn alpha_numeric_rule_accepts_letters_and_digits() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("username", "user123")
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
    async fn digits_rule_rejects_non_digits() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("zip", "12a45").digits().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "digits");
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
    async fn timezone_rule_accepts_iana() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("tz", "Asia/Kuala_Lumpur")
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
        for value in ["abc", "NaN", "inf", "-inf", "1.2.3", ""] {
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
    async fn integer_rejects_decimal() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("count", "3.14").integer().apply().await.unwrap();
        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].code, "integer");
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

    // --- Ipv4 rule tests ---

    #[tokio::test]
    async fn ipv4_accepts_valid() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("ip", "192.168.1.1").ipv4().apply().await.unwrap();
        assert!(v.finish().is_ok());
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
    async fn ipv6_rejects_ipv4() {
        let app = test_app();
        let mut v = Validator::new(app);
        v.field("ip", "192.168.1.1").ipv6().apply().await.unwrap();
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
                    "mobile": "The translated {{attribute}} is invalid.",
                    "attributes": {
                        "email": "email address"
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
    async fn named_rule_without_override_preserves_returned_message() {
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
        assert_eq!(errors.errors[0].message, "invalid mobile number");
    }

    #[tokio::test]
    async fn named_rule_i18n_message_precedes_returned_message() {
        let (app, _dir) = test_app_with_i18n();
        app.rules()
            .register(ValidationRuleId::new("mobile"), MobileRule)
            .unwrap();
        let mut v = Validator::new(app);
        v.field("phone", "123")
            .rule(ValidationRuleId::new("mobile"))
            .apply()
            .await
            .unwrap();

        let errors = v.finish().unwrap_err();
        assert_eq!(errors.errors[0].message, "The translated phone is invalid.");
    }

    #[tokio::test]
    async fn named_rule_returned_message_interpolates_attribute() {
        let rules = RuleRegistry::new();
        rules
            .register(ValidationRuleId::new("placeholder"), PlaceholderRule)
            .unwrap();
        let app = AppContext::new(Container::new(), ConfigRepository::empty(), rules).unwrap();
        let mut v = Validator::new(app);
        v.custom_attribute("account_code", "account code");
        v.field("account_code", "bad")
            .rule(ValidationRuleId::new("placeholder"))
            .apply()
            .await
            .unwrap();

        let errors = v.finish().unwrap_err();
        assert_eq!(
            errors.errors[0].message,
            "The account code failed its custom rule."
        );
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

    #[tokio::test]
    async fn conditional_required_rules_only_require_values_when_the_condition_matches() {
        let app = test_app();
        let mut validator = Validator::new(app);

        validator
            .optional_field("publishedAt", Option::<String>::None)
            .required_if("status", "scheduled", ["published", "scheduled"])
            .apply()
            .await
            .unwrap();
        validator
            .optional_field("reviewNote", Option::<String>::None)
            .required_unless("status", "draft", ["draft"])
            .apply()
            .await
            .unwrap();
        validator
            .optional_field("phone", Option::<String>::None)
            .required_with([("email", "person@example.com")])
            .apply()
            .await
            .unwrap();

        let errors = validator.finish().unwrap_err();
        assert_eq!(
            errors
                .errors
                .iter()
                .map(|error| error.code.as_str())
                .collect::<Vec<_>>(),
            vec!["required_if", "required_with"]
        );
    }

    #[tokio::test]
    async fn presence_rules_distinguish_absent_present_and_empty_values() {
        let app = test_app();
        let mut validator = Validator::new(app);

        validator
            .optional_field("presentField", Option::<String>::None)
            .present()
            .apply()
            .await
            .unwrap();
        validator
            .optional_field("skippedEmail", Option::<String>::None)
            .sometimes()
            .email()
            .apply()
            .await
            .unwrap();
        validator
            .optional_field("invalidEmail", Some("not-an-email"))
            .sometimes()
            .email()
            .apply()
            .await
            .unwrap();
        validator
            .optional_field("forbidden", Some("supplied"))
            .prohibited()
            .apply()
            .await
            .unwrap();
        validator
            .optional_field("emptyForbidden", Some(""))
            .prohibited()
            .apply()
            .await
            .unwrap();

        let errors = validator.finish().unwrap_err();
        assert_eq!(
            errors
                .errors
                .iter()
                .map(|error| error.code.as_str())
                .collect::<Vec<_>>(),
            vec!["present", "email", "prohibited"]
        );
    }

    #[tokio::test]
    async fn boolean_rule_accepts_boolean_lexemes_and_rejects_other_values() {
        for value in ["true", "false", "1", "0"] {
            let mut validator = Validator::new(test_app());
            validator
                .field("enabled", value)
                .boolean()
                .apply()
                .await
                .unwrap();
            assert!(validator.finish().is_ok(), "value: {value}");
        }

        let mut validator = Validator::new(test_app());
        validator
            .field("enabled", "yes")
            .boolean()
            .apply()
            .await
            .unwrap();
        assert_eq!(validator.finish().unwrap_err().errors[0].code, "boolean");
    }

    #[tokio::test]
    async fn distinct_rule_validates_the_collection_as_a_whole() {
        let unique = vec!["rust", "foundry"];
        let mut validator = Validator::new(test_app());
        validator
            .each("tags", &unique)
            .distinct()
            .apply()
            .await
            .unwrap();
        assert!(validator.finish().is_ok());

        let duplicate = vec!["rust", "foundry", "rust"];
        let mut validator = Validator::new(test_app());
        validator
            .each("tags", &duplicate)
            .distinct()
            .apply()
            .await
            .unwrap();
        let errors = validator.finish().unwrap_err();
        assert_eq!(errors.errors.len(), 1);
        assert_eq!(errors.errors[0].field, "tags");
        assert_eq!(errors.errors[0].code, "distinct");
    }
}
