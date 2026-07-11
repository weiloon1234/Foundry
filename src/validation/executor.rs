use std::net::IpAddr;

use regex::Regex;
use url::Url;
use uuid::Uuid;
use validator::ValidateEmail;

use crate::database::Query;
use crate::foundation::{AppContext, Error, Result};
use crate::logging::{catch_async_panic, panic_payload_message};
use crate::support::ValidationRuleId;
use crate::support::{Date, DateTime, LocalDateTime, Time, Timezone};
use crate::validation::context::{RuleContext, ValidationRule};
use crate::validation::rules::{FieldRule, FieldStep};
use crate::validation::types::{FieldError, ValidationError};
use crate::validation::validator::Validator;

pub(crate) fn parse_datetime_value(app: &AppContext, value: &str) -> Result<DateTime> {
    DateTime::parse_in_timezone(value, &app.timezone()?)
}

pub(crate) fn parse_local_datetime_value(value: &str) -> Result<LocalDateTime> {
    if DateTime::parse(value).is_ok() {
        return Err(Error::message(
            "offset-aware datetimes are not valid local datetime values",
        ));
    }
    LocalDateTime::parse(value)
}

pub(crate) fn compare_temporal_values(
    app: &AppContext,
    left: &str,
    right: &str,
) -> Result<std::cmp::Ordering> {
    if let (Ok(left), Ok(right)) = (
        parse_datetime_value(app, left),
        parse_datetime_value(app, right),
    ) {
        return Ok(left.cmp(&right));
    }

    if let (Ok(left), Ok(right)) = (
        parse_local_datetime_value(left),
        parse_local_datetime_value(right),
    ) {
        return Ok(left.cmp(&right));
    }

    if let (Ok(left), Ok(right)) = (Date::parse(left), Date::parse(right)) {
        return Ok(left.cmp(&right));
    }

    if let (Ok(left), Ok(right)) = (Time::parse(left), Time::parse(right)) {
        return Ok(left.cmp(&right));
    }

    Err(Error::message(
        "values are not comparable date/time strings",
    ))
}

/// Parses a value for numeric rules, rejecting NaN/infinity: every IEEE 754
/// comparison against NaN is false, so a bare `parse` would let "NaN" sail
/// through min/max/between bound checks.
pub(crate) fn parse_finite_number(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|num| num.is_finite())
}

pub(crate) fn interpolate_message(template: &str, values: &[(&str, &str)]) -> String {
    let mut result = template.to_string();
    for (key, value) in values {
        let placeholder = format!("{{{{{}}}}}", key);
        result = result.replace(&placeholder, value);
    }
    result
}

pub(crate) fn fallback_message(field: &str, code: &str, params: &[(&str, &str)]) -> String {
    let get_param = |name: &str| -> &str {
        params
            .iter()
            .find(|(k, _)| k == &name)
            .map(|(_, v)| *v)
            .unwrap_or("")
    };
    match code {
        "required" => format!("The {} field is required.", field),
        "required_if" => format!(
            "The {} field is required when {} is {}.",
            field,
            get_param("other"),
            get_param("values")
        ),
        "required_unless" => format!(
            "The {} field is required unless {} is {}.",
            field,
            get_param("other"),
            get_param("values")
        ),
        "required_with" => format!(
            "The {} field is required when {} is present.",
            field,
            get_param("other")
        ),
        "present" => format!("The {} field must be present.", field),
        "prohibited" => format!("The {} field is prohibited.", field),
        "email" => format!("The {} must be a valid email address.", field),
        "min" => format!(
            "The {} must be at least {} characters.",
            field,
            get_param("min")
        ),
        "max" => format!(
            "The {} must not exceed {} characters.",
            field,
            get_param("max")
        ),
        "numeric" => format!("The {} must be a number.", field),
        "boolean" => format!("The {} field must be true or false.", field),
        "integer" => format!("The {} must be an integer.", field),
        "alpha" => format!("The {} must contain only letters.", field),
        "alpha_numeric" => format!("The {} must contain only letters and numbers.", field),
        "digits" => format!("The {} must contain only digits.", field),
        "url" => format!("The {} must be a valid URL.", field),
        "uuid" => format!("The {} must be a valid UUID.", field),
        "regex" => format!("The {} has an invalid format.", field),
        "json" => format!("The {} must be valid JSON.", field),
        "timezone" => format!("The {} must be a valid timezone.", field),
        "ip" => format!("The {} must be a valid IP address.", field),
        "ipv4" => format!("The {} must be a valid IPv4 address.", field),
        "ipv6" => format!("The {} must be a valid IPv6 address.", field),
        "date" => format!("The {} must be a valid date.", field),
        "time" => format!("The {} must be a valid time.", field),
        "datetime" => format!("The {} must be a valid datetime.", field),
        "local_datetime" => format!("The {} must be a valid local datetime.", field),
        "in_list" => format!("The selected {} is invalid.", field),
        "not_in" => format!("The {} has an invalid value.", field),
        "starts_with" => format!("The {} must start with {}.", field, get_param("value")),
        "ends_with" => format!("The {} must end with {}.", field, get_param("value")),
        "confirmed" => format!("The {} confirmation does not match.", field),
        "same" => format!("The {} must match {}.", field, get_param("other")),
        "different" => format!(
            "The {} must be different from {}.",
            field,
            get_param("other")
        ),
        "before" => format!(
            "The {} must be a date before {}.",
            field,
            get_param("other")
        ),
        "before_or_equal" => format!(
            "The {} must be a date before or equal to {}.",
            field,
            get_param("other")
        ),
        "after" => format!("The {} must be a date after {}.", field, get_param("other")),
        "after_or_equal" => format!(
            "The {} must be a date after or equal to {}.",
            field,
            get_param("other")
        ),
        "min_numeric" => format!("The {} must be at least {}.", field, get_param("min")),
        "max_numeric" => format!("The {} must not exceed {}.", field, get_param("max")),
        "between" => format!(
            "The {} must be between {} and {}.",
            field,
            get_param("min"),
            get_param("max")
        ),
        "unique" => format!("The {} has already been taken.", field),
        "exists" => format!("The selected {} is invalid.", field),
        "app_enum" => format!("The selected {} is invalid.", field),
        "distinct" => format!("The {} field has a duplicate value.", field),
        "image" => format!("The {} must be an image.", field),
        "max_file_size" => format!("The {} must not exceed {}KB.", field, get_param("max")),
        "max_dimensions" => format!(
            "The {} dimensions must not exceed {}x{} pixels.",
            field,
            get_param("width"),
            get_param("height")
        ),
        "min_dimensions" => format!(
            "The {} must be at least {}x{} pixels.",
            field,
            get_param("width"),
            get_param("height")
        ),
        "allowed_mimes" => format!("The {} file type is not allowed.", field),
        "allowed_extensions" => format!("The {} file extension is not allowed.", field),
        "invalid_request_body" => "The request body is invalid.".to_string(),
        "multipart_not_supported" => {
            "Multipart form-data is not supported for this endpoint.".to_string()
        }
        _ => format!("The {} is invalid.", field),
    }
}

pub(crate) async fn execute_steps(
    validator: &mut Validator,
    field: &str,
    value: &str,
    present: bool,
    steps: Vec<FieldStep>,
    nullable: bool,
    bail: bool,
) -> Result<()> {
    for step in steps {
        if nullable && value.trim().is_empty() && !rule_runs_when_empty(&step.rule) {
            continue;
        }
        let errors_before = validator.errors.len();
        match step {
            FieldStep {
                rule: FieldRule::Required,
                message,
            } => {
                if !present || value.trim().is_empty() {
                    let msg = validator.resolve_message(field, "required", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("required", msg));
                    // Required failing on empty value implies no further rules can meaningfully run
                    break;
                }
            }
            FieldStep {
                rule:
                    FieldRule::RequiredIf {
                        other_field,
                        other_value,
                        expected_values,
                    },
                message,
            } => {
                if expected_values.contains(&other_value) && (!present || value.trim().is_empty()) {
                    let values = expected_values.join(", ");
                    let msg = validator.resolve_message(
                        field,
                        "required_if",
                        &[("other", &other_field), ("values", &values)],
                        message.as_deref(),
                    );
                    validator
                        .push_error(field.to_string(), ValidationError::new("required_if", msg));
                    break;
                }
            }
            FieldStep {
                rule:
                    FieldRule::RequiredUnless {
                        other_field,
                        other_value,
                        expected_values,
                    },
                message,
            } => {
                if !expected_values.contains(&other_value) && (!present || value.trim().is_empty())
                {
                    let values = expected_values.join(", ");
                    let msg = validator.resolve_message(
                        field,
                        "required_unless",
                        &[("other", &other_field), ("values", &values)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("required_unless", msg),
                    );
                    break;
                }
            }
            FieldStep {
                rule: FieldRule::RequiredWith { other_fields },
                message,
            } => {
                let active_fields = other_fields
                    .iter()
                    .filter_map(|(name, value)| (!value.trim().is_empty()).then_some(name.as_str()))
                    .collect::<Vec<_>>();
                if !active_fields.is_empty() && (!present || value.trim().is_empty()) {
                    let other = active_fields.join(", ");
                    let msg = validator.resolve_message(
                        field,
                        "required_with",
                        &[("other", &other)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("required_with", msg),
                    );
                    break;
                }
            }
            FieldStep {
                rule: FieldRule::Present,
                message,
            } => {
                if !present {
                    let msg = validator.resolve_message(field, "present", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("present", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Prohibited,
                message,
            } => {
                if present && !value.trim().is_empty() {
                    let msg =
                        validator.resolve_message(field, "prohibited", &[], message.as_deref());
                    validator
                        .push_error(field.to_string(), ValidationError::new("prohibited", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Email,
                message,
            } => {
                if !value.validate_email() {
                    let msg = validator.resolve_message(field, "email", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("email", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Min(length),
                message,
            } => {
                if value.chars().count() < length {
                    let msg = validator.resolve_message(
                        field,
                        "min",
                        &[("min", &length.to_string())],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("min", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Max(length),
                message,
            } => {
                if value.chars().count() > length {
                    let msg = validator.resolve_message(
                        field,
                        "max",
                        &[("max", &length.to_string())],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("max", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Named(id),
                message,
            } => {
                let Some(rule) = validator.app.rules().get(&id)? else {
                    return Err(Error::message(format!(
                        "validation rule `{id}` is not registered"
                    )));
                };
                let context = RuleContext::new(validator.app.clone(), field.to_string());
                if let Err(error) = run_named_rule(&id, rule.as_ref(), &context, value).await? {
                    let msg = validator.resolve_named_rule_message(
                        field,
                        &error.code,
                        &error.message,
                        message.as_deref(),
                    );
                    validator.errors.push(FieldError {
                        field: field.to_string(),
                        code: error.code,
                        message: msg,
                    });
                }
            }
            FieldStep {
                rule: FieldRule::Regex(pattern),
                message,
            } => {
                let re = Regex::new(&pattern)
                    .map_err(|e| Error::message(format!("invalid regex pattern: {e}")))?;
                if !re.is_match(value) {
                    let msg = validator.resolve_message(field, "regex", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("regex", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Url,
                message,
            } => {
                if value.is_empty() || Url::parse(value).is_err() {
                    let msg = validator.resolve_message(field, "url", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("url", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Uuid,
                message,
            } => {
                if value.parse::<Uuid>().is_err() {
                    let msg = validator.resolve_message(field, "uuid", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("uuid", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Numeric,
                message,
            } => {
                if parse_finite_number(value).is_none() {
                    let msg = validator.resolve_message(field, "numeric", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("numeric", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Boolean,
                message,
            } => {
                if !matches!(value, "true" | "false" | "1" | "0") {
                    let msg = validator.resolve_message(field, "boolean", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("boolean", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Alpha,
                message,
            } => {
                if !value.chars().all(|c| c.is_alphabetic()) {
                    let msg = validator.resolve_message(field, "alpha", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("alpha", msg));
                }
            }
            FieldStep {
                rule: FieldRule::AlphaNumeric,
                message,
            } => {
                if !value.chars().all(|c| c.is_alphanumeric()) {
                    let msg =
                        validator.resolve_message(field, "alpha_numeric", &[], message.as_deref());
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("alpha_numeric", msg),
                    );
                }
            }
            FieldStep {
                rule: FieldRule::InList(values),
                message,
            } => {
                if !values.contains(&value.to_string()) {
                    let msg = validator.resolve_message(field, "in_list", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("in_list", msg));
                }
            }
            FieldStep {
                rule: FieldRule::NotIn(values),
                message,
            } => {
                if values.contains(&value.to_string()) {
                    let msg = validator.resolve_message(field, "not_in", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("not_in", msg));
                }
            }
            FieldStep {
                rule: FieldRule::StartsWith(prefix),
                message,
            } => {
                if !value.starts_with(prefix.as_str()) {
                    let msg = validator.resolve_message(
                        field,
                        "starts_with",
                        &[("value", &prefix)],
                        message.as_deref(),
                    );
                    validator
                        .push_error(field.to_string(), ValidationError::new("starts_with", msg));
                }
            }
            FieldStep {
                rule: FieldRule::EndsWith(suffix),
                message,
            } => {
                if !value.ends_with(suffix.as_str()) {
                    let msg = validator.resolve_message(
                        field,
                        "ends_with",
                        &[("value", &suffix)],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("ends_with", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Ip,
                message,
            } => {
                if value.parse::<IpAddr>().is_err() {
                    let msg = validator.resolve_message(field, "ip", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("ip", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Json,
                message,
            } => {
                if serde_json::from_str::<serde_json::Value>(value).is_err() {
                    let msg = validator.resolve_message(field, "json", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("json", msg));
                }
            }
            FieldStep {
                rule:
                    FieldRule::Confirmed {
                        other_field,
                        other_value,
                    },
                message,
            } => {
                if value != other_value {
                    let msg = validator.resolve_message(
                        field,
                        "confirmed",
                        &[("other", &other_field)],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("confirmed", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Digits,
                message,
            } => {
                if !value.chars().all(|c| c.is_ascii_digit()) {
                    let msg = validator.resolve_message(field, "digits", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("digits", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Timezone,
                message,
            } => {
                if Timezone::parse(value).is_err() {
                    let msg = validator.resolve_message(field, "timezone", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("timezone", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Date,
                message,
            } => {
                if Date::parse(value).is_err() {
                    let msg = validator.resolve_message(field, "date", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("date", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Time,
                message,
            } => {
                if Time::parse(value).is_err() {
                    let msg = validator.resolve_message(field, "time", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("time", msg));
                }
            }
            FieldStep {
                rule: FieldRule::DateTime,
                message,
            } => {
                if parse_datetime_value(validator.app(), value).is_err() {
                    let msg = validator.resolve_message(field, "datetime", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("datetime", msg));
                }
            }
            FieldStep {
                rule: FieldRule::LocalDateTime,
                message,
            } => {
                if parse_local_datetime_value(value).is_err() {
                    let msg =
                        validator.resolve_message(field, "local_datetime", &[], message.as_deref());
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("local_datetime", msg),
                    );
                }
            }
            FieldStep {
                rule:
                    FieldRule::Before {
                        other_field,
                        other_value,
                        allow_equal,
                    },
                message,
            } => {
                let code = if allow_equal {
                    "before_or_equal"
                } else {
                    "before"
                };
                match compare_temporal_values(validator.app(), value, &other_value) {
                    Ok(ordering) if ordering.is_lt() || (allow_equal && ordering.is_eq()) => {}
                    Ok(_) => {
                        let msg = validator.resolve_message(
                            field,
                            code,
                            &[("other", &other_field)],
                            message.as_deref(),
                        );
                        validator.push_error(field.to_string(), ValidationError::new(code, msg));
                    }
                    Err(_) => {
                        let msg = validator.resolve_message(
                            field,
                            code,
                            &[("other", &other_field)],
                            message.as_deref(),
                        );
                        validator.push_error(field.to_string(), ValidationError::new(code, msg));
                    }
                }
            }
            FieldStep {
                rule:
                    FieldRule::After {
                        other_field,
                        other_value,
                        allow_equal,
                    },
                message,
            } => {
                let code = if allow_equal {
                    "after_or_equal"
                } else {
                    "after"
                };
                match compare_temporal_values(validator.app(), value, &other_value) {
                    Ok(ordering) if ordering.is_gt() || (allow_equal && ordering.is_eq()) => {}
                    Ok(_) => {
                        let msg = validator.resolve_message(
                            field,
                            code,
                            &[("other", &other_field)],
                            message.as_deref(),
                        );
                        validator.push_error(field.to_string(), ValidationError::new(code, msg));
                    }
                    Err(_) => {
                        let msg = validator.resolve_message(
                            field,
                            code,
                            &[("other", &other_field)],
                            message.as_deref(),
                        );
                        validator.push_error(field.to_string(), ValidationError::new(code, msg));
                    }
                }
            }
            FieldStep {
                rule: FieldRule::MinNumeric(min),
                message,
            } => {
                if !matches!(parse_finite_number(value), Some(num) if num >= min) {
                    let msg = validator.resolve_message(
                        field,
                        "min_numeric",
                        &[("min", &min.to_string())],
                        message.as_deref(),
                    );
                    validator
                        .push_error(field.to_string(), ValidationError::new("min_numeric", msg));
                }
            }
            FieldStep {
                rule: FieldRule::MaxNumeric(max),
                message,
            } => {
                if !matches!(parse_finite_number(value), Some(num) if num <= max) {
                    let msg = validator.resolve_message(
                        field,
                        "max_numeric",
                        &[("max", &max.to_string())],
                        message.as_deref(),
                    );
                    validator
                        .push_error(field.to_string(), ValidationError::new("max_numeric", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Integer,
                message,
            } => {
                if value.parse::<i64>().is_err() {
                    let msg = validator.resolve_message(field, "integer", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("integer", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Between { min, max },
                message,
            } => {
                if !matches!(parse_finite_number(value), Some(num) if num >= min && num <= max) {
                    let msg = validator.resolve_message(
                        field,
                        "between",
                        &[("min", &min.to_string()), ("max", &max.to_string())],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("between", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Ipv4,
                message,
            } => {
                if value.parse::<std::net::Ipv4Addr>().is_err() {
                    let msg = validator.resolve_message(field, "ipv4", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("ipv4", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Ipv6,
                message,
            } => {
                if value.parse::<std::net::Ipv6Addr>().is_err() {
                    let msg = validator.resolve_message(field, "ipv6", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("ipv6", msg));
                }
            }
            FieldStep {
                rule:
                    FieldRule::Same {
                        other_field,
                        other_value,
                    },
                message,
            } => {
                if value != other_value {
                    let msg = validator.resolve_message(
                        field,
                        "same",
                        &[("other", &other_field)],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("same", msg));
                }
            }
            FieldStep {
                rule:
                    FieldRule::Different {
                        other_field,
                        other_value,
                    },
                message,
            } => {
                if value == other_value {
                    let msg = validator.resolve_message(
                        field,
                        "different",
                        &[("other", &other_field)],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("different", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Unique { table, column },
                message,
            } => {
                let db = validator.app().database()?;
                let count = Query::table(table.as_str())
                    .where_eq(column.as_str(), value)
                    .count(db.as_ref())
                    .await
                    .map_err(|e| Error::message(format!("unique validation query failed: {e}")))?;
                if count > 0 {
                    let msg = validator.resolve_message(field, "unique", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("unique", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Exists { table, column },
                message,
            } => {
                let db = validator.app().database()?;
                let count = Query::table(table.as_str())
                    .where_eq(column.as_str(), value)
                    .count(db.as_ref())
                    .await
                    .map_err(|e| Error::message(format!("exists validation query failed: {e}")))?;
                if count == 0 {
                    let msg = validator.resolve_message(field, "exists", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("exists", msg));
                }
            }
            FieldStep {
                rule: FieldRule::AppEnum { valid_keys },
                message,
            } => {
                if !valid_keys.contains(&value.to_string()) {
                    let msg = validator.resolve_message(field, "app_enum", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("app_enum", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Distinct,
                ..
            } => {
                return Err(Error::message(
                    "distinct validation requires Validator::each",
                ));
            }
        }
        if bail && validator.errors.len() > errors_before {
            break;
        }
    }
    Ok(())
}

fn rule_runs_when_empty(rule: &FieldRule) -> bool {
    matches!(
        rule,
        FieldRule::RequiredIf { .. }
            | FieldRule::RequiredUnless { .. }
            | FieldRule::RequiredWith { .. }
            | FieldRule::Present
            | FieldRule::Prohibited
            | FieldRule::Distinct
    )
}

async fn run_named_rule(
    id: &ValidationRuleId,
    rule: &dyn ValidationRule,
    context: &RuleContext,
    value: &str,
) -> Result<std::result::Result<(), ValidationError>> {
    match catch_async_panic(|| rule.validate(context, value)).await {
        Ok(result) => Ok(result),
        Err(panic) => {
            let message = panic_payload_message(panic);
            tracing::error!(
                target: "foundry.validation",
                rule = %id,
                field = %context.field(),
                panic = %message,
                "validation rule panicked"
            );
            Err(Error::message(format!(
                "validation rule `{id}` panicked: {message}"
            )))
        }
    }
}
