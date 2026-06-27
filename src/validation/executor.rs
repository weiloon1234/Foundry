use std::{net::IpAddr, sync::LazyLock};

use regex::Regex;
use url::Url;
use uuid::Uuid;

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

pub(crate) fn decimal_places(value: &str) -> Option<usize> {
    let text = value.trim();
    if text.contains('e') || text.contains('E') || parse_finite_number(text).is_none() {
        return None;
    }

    let unsigned = text
        .strip_prefix('+')
        .or_else(|| text.strip_prefix('-'))
        .unwrap_or(text);
    let (whole, fraction) = unsigned.split_once('.')?;
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    if (!whole.is_empty() && !whole.chars().all(|ch| ch.is_ascii_digit()))
        || !fraction.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }

    Some(fraction.len())
}

pub(crate) fn has_decimal_places(value: &str, min: usize, max: usize) -> bool {
    min <= max && matches!(decimal_places(value), Some(places) if places >= min && places <= max)
}

pub(crate) fn has_digit_length(value: &str, min: usize, max: usize) -> bool {
    min <= max && value.chars().all(|ch| ch.is_ascii_digit()) && {
        let length = value.chars().count();
        length >= min && length <= max
    }
}

pub(crate) fn is_url_value(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(char::is_whitespace) && Url::parse(value).is_ok()
}

pub(crate) fn is_alpha_value(value: &str) -> bool {
    static ALPHA: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[\p{L}\p{M}]*$").expect("valid alpha regex"));
    ALPHA.is_match(value)
}

pub(crate) fn is_alpha_dash_value(value: &str) -> bool {
    static ALPHA_DASH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[\p{L}\p{M}\p{N}_-]*$").expect("valid alpha_dash regex"));
    ALPHA_DASH.is_match(value)
}

pub(crate) fn is_alpha_numeric_value(value: &str) -> bool {
    static ALPHA_NUMERIC: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[\p{L}\p{M}\p{N}]*$").expect("valid alpha_num regex"));
    ALPHA_NUMERIC.is_match(value)
}

pub(crate) fn is_multiple_of_value(value: &str, divisor: f64) -> bool {
    if !divisor.is_finite() || divisor <= 0.0 {
        return false;
    }

    let Some(number) = parse_finite_number(value) else {
        return false;
    };

    let quotient = number / divisor;
    let nearest = quotient.round();
    let tolerance = f64::EPSILON * 16.0 * quotient.abs().max(1.0);
    (quotient - nearest).abs() <= tolerance
}

pub(crate) fn is_same_number_value(value: &str, expected: f64) -> bool {
    if !expected.is_finite() {
        return false;
    }

    let Some(number) = parse_finite_number(value) else {
        return false;
    };

    let tolerance = f64::EPSILON * 16.0 * number.abs().max(expected.abs()).max(1.0);
    (number - expected).abs() <= tolerance
}

pub(crate) fn is_boolean_value(value: &str) -> bool {
    matches!(value.trim(), "true" | "false" | "1" | "0")
}

pub(crate) fn is_accepted_value(value: &str) -> bool {
    matches!(value.trim(), "yes" | "on" | "1" | "true")
}

pub(crate) fn is_declined_value(value: &str) -> bool {
    matches!(value.trim(), "no" | "off" | "0" | "false")
}

pub(crate) fn is_ulid_value(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 26 {
        return false;
    }

    matches!(bytes[0], b'0'..=b'7')
        && bytes[1..]
            .iter()
            .all(|byte| is_crockford_base32(byte.to_ascii_uppercase()))
}

pub(crate) fn is_uuid_value(value: &str, version: Option<u8>) -> bool {
    let Ok(uuid) = value.parse::<Uuid>() else {
        return false;
    };

    version.is_none_or(|version| uuid.get_version_num() == usize::from(version))
}

pub(crate) fn is_email_value(value: &str) -> bool {
    let Some((user_part, domain_part)) = value.rsplit_once('@') else {
        return false;
    };

    if value.is_empty()
        || user_part.is_empty()
        || domain_part.is_empty()
        || user_part.chars().count() > 64
        || domain_part.chars().count() > 255
    {
        return false;
    }

    static EMAIL_USER_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+\z").unwrap());
    if !EMAIL_USER_RE.is_match(user_part) {
        return false;
    }

    if is_email_domain_part(domain_part) {
        return true;
    }

    let ascii_domain = url::quirks::domain_to_ascii(domain_part);
    !ascii_domain.is_empty() && is_email_domain_part(&ascii_domain)
}

fn is_email_domain_part(domain: &str) -> bool {
    static EMAIL_DOMAIN_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"^[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$",
        )
        .unwrap()
    });
    if EMAIL_DOMAIN_RE.is_match(domain) {
        return true;
    }

    let Some(literal) = domain
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    else {
        return false;
    };

    literal.parse::<IpAddr>().is_ok()
}

fn is_crockford_base32(byte: u8) -> bool {
    matches!(
        byte,
        b'0'..=b'9' | b'A'..=b'H' | b'J'..=b'K' | b'M'..=b'N' | b'P'..=b'T' | b'V'..=b'Z'
    )
}

pub(crate) fn is_hex_color_value(value: &str) -> bool {
    let Some(hex) = value.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|ch| ch.is_ascii_hexdigit())
}

pub(crate) fn is_mac_address_value(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 17 {
        return false;
    }

    let separator = bytes[2];
    if !matches!(separator, b':' | b'-') {
        return false;
    }

    for (index, byte) in bytes.iter().copied().enumerate() {
        if matches!(index, 2 | 5 | 8 | 11 | 14) {
            if byte != separator {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }

    true
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
        "filled" => format!("The {} field must have a value.", field),
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
        "size" => format!("The {} must be exactly {}.", field, get_param("size")),
        "numeric" => format!("The {} must be a number.", field),
        "decimal" if get_param("min") == get_param("max") => format!(
            "The {} must have {} decimal places.",
            field,
            get_param("min")
        ),
        "decimal" => format!(
            "The {} must have between {} and {} decimal places.",
            field,
            get_param("min"),
            get_param("max")
        ),
        "multiple_of" => format!(
            "The {} must be a multiple of {}.",
            field,
            get_param("value")
        ),
        "boolean" => format!("The {} must be true or false.", field),
        "accepted" => format!("The {} must be accepted.", field),
        "accepted_if" => format!(
            "The {} must be accepted when {} is {}.",
            field,
            get_param("other"),
            get_param("value")
        ),
        "declined" => format!("The {} must be declined.", field),
        "declined_if" => format!(
            "The {} must be declined when {} is {}.",
            field,
            get_param("other"),
            get_param("value")
        ),
        "prohibited" => format!("The {} field is prohibited.", field),
        "prohibited_if" => format!(
            "The {} field is prohibited when {} is {}.",
            field,
            get_param("other"),
            get_param("value")
        ),
        "prohibited_unless" => format!(
            "The {} field is prohibited unless {} is {}.",
            field,
            get_param("other"),
            get_param("value")
        ),
        "prohibited_if_accepted" => format!(
            "The {} field is prohibited when {} is accepted.",
            field,
            get_param("other")
        ),
        "prohibited_if_declined" => format!(
            "The {} field is prohibited when {} is declined.",
            field,
            get_param("other")
        ),
        "prohibits" => format!("The {} field prohibits {}.", field, get_param("other")),
        "integer" => format!("The {} must be an integer.", field),
        "alpha" => format!("The {} must contain only letters.", field),
        "alpha_dash" => format!(
            "The {} must contain only letters, numbers, dashes, and underscores.",
            field
        ),
        "alpha_num" => format!("The {} must contain only letters and numbers.", field),
        "alpha_numeric" => format!("The {} must contain only letters and numbers.", field),
        "ascii" => format!("The {} must only contain ASCII characters.", field),
        "lowercase" => format!("The {} must be lowercase.", field),
        "uppercase" => format!("The {} must be uppercase.", field),
        "digits" => format!("The {} must contain only digits.", field),
        "min_digits" => format!(
            "The {} must have at least {} digits.",
            field,
            get_param("min")
        ),
        "max_digits" => format!(
            "The {} must not have more than {} digits.",
            field,
            get_param("max")
        ),
        "digits_between" => format!(
            "The {} must have between {} and {} digits.",
            field,
            get_param("min"),
            get_param("max")
        ),
        "url" => format!("The {} must be a valid URL.", field),
        "uuid" if get_param("version").is_empty() => {
            format!("The {} must be a valid UUID.", field)
        }
        "uuid" => format!(
            "The {} must be a valid version {} UUID.",
            field,
            get_param("version")
        ),
        "ulid" => format!("The {} must be a valid ULID.", field),
        "hex_color" => format!("The {} must be a valid hexadecimal color.", field),
        "mac_address" => format!("The {} must be a valid MAC address.", field),
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
        "doesnt_start_with" => format!("The {} must not start with {}.", field, get_param("value")),
        "ends_with" => format!("The {} must end with {}.", field, get_param("value")),
        "doesnt_end_with" => format!("The {} must not end with {}.", field, get_param("value")),
        "contains" => format!("The {} must contain {}.", field, get_param("value")),
        "doesnt_contain" => format!("The {} must not contain {}.", field, get_param("value")),
        "min_items" => format!(
            "The {} must contain at least {} items.",
            field,
            get_param("min")
        ),
        "max_items" => format!(
            "The {} must not contain more than {} items.",
            field,
            get_param("max")
        ),
        "distinct" => format!("The {} must not contain duplicate items.", field),
        "required_if" => format!(
            "The {} field is required when {} is {}.",
            field,
            get_param("other"),
            get_param("value")
        ),
        "required_unless" => format!(
            "The {} field is required unless {} is {}.",
            field,
            get_param("other"),
            get_param("value")
        ),
        "required_if_accepted" => format!(
            "The {} field is required when {} is accepted.",
            field,
            get_param("other")
        ),
        "required_if_declined" => format!(
            "The {} field is required when {} is declined.",
            field,
            get_param("other")
        ),
        "required_with" => format!(
            "The {} field is required when {} is present.",
            field,
            get_param("other")
        ),
        "required_with_all" => format!(
            "The {} field is required when {} are present.",
            field,
            get_param("other")
        ),
        "required_without" => format!(
            "The {} field is required when {} is not present.",
            field,
            get_param("other")
        ),
        "required_without_all" => format!(
            "The {} field is required when none of {} are present.",
            field,
            get_param("other")
        ),
        "required_keys" => format!(
            "The {} field must contain entries for {}.",
            field,
            get_param("keys")
        ),
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
        "date_equals" => format!(
            "The {} must be a date equal to {}.",
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
        "gt" => format!("The {} must be greater than {}.", field, get_param("value")),
        "gte" => format!(
            "The {} must be greater than or equal to {}.",
            field,
            get_param("value")
        ),
        "lt" => format!("The {} must be less than {}.", field, get_param("value")),
        "lte" => format!(
            "The {} must be less than or equal to {}.",
            field,
            get_param("value")
        ),
        "not_regex" => format!("The {} has an invalid format.", field),
        "unique" => format!("The {} has already been taken.", field),
        "exists" => format!("The selected {} is invalid.", field),
        "app_enum" => format!("The selected {} is invalid.", field),
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
    steps: Vec<FieldStep>,
    bail: bool,
) -> Result<()> {
    for step in steps {
        let errors_before = validator.errors.len();
        match step {
            FieldStep {
                rule: FieldRule::Required,
                message,
            } => {
                if value.trim().is_empty() {
                    let msg = validator.resolve_message(field, "required", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("required", msg));
                    // Required failing on empty value implies no further rules can meaningfully run
                    break;
                }
            }
            FieldStep {
                rule: FieldRule::Filled,
                message,
            } => {
                if value.trim().is_empty() {
                    let msg = validator.resolve_message(field, "filled", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("filled", msg));
                    break;
                }
            }
            FieldStep {
                rule: FieldRule::FilledCollection,
                message,
            } => {
                if value.trim().is_empty() {
                    let msg = validator.resolve_message(field, "filled", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("filled", msg));
                    break;
                }
            }
            FieldStep {
                rule: FieldRule::Email,
                message,
            } => {
                if !is_email_value(value) {
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
                rule: FieldRule::Size(size),
                message,
            } => {
                if value.chars().count() != size {
                    let msg = validator.resolve_message(
                        field,
                        "size",
                        &[("size", &size.to_string())],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("size", msg));
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
                    let msg = match message.as_deref() {
                        Some(custom) => custom.to_string(),
                        None => validator.resolve_message(field, &error.code, &[], None),
                    };
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
                rule: FieldRule::NotRegex(pattern),
                message,
            } => {
                let re = Regex::new(&pattern)
                    .map_err(|e| Error::message(format!("invalid regex pattern: {e}")))?;
                if re.is_match(value) {
                    let msg =
                        validator.resolve_message(field, "not_regex", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("not_regex", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Url,
                message,
            } => {
                if !is_url_value(value) {
                    let msg = validator.resolve_message(field, "url", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("url", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Uuid { version },
                message,
            } => {
                if !is_uuid_value(value, version) {
                    let version_param = version.map(|version| version.to_string());
                    let params = version_param
                        .as_ref()
                        .map(|version| vec![("version", version.as_str())])
                        .unwrap_or_default();
                    let msg = validator.resolve_message(field, "uuid", &params, message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("uuid", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Ulid,
                message,
            } => {
                if !is_ulid_value(value) {
                    let msg = validator.resolve_message(field, "ulid", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("ulid", msg));
                }
            }
            FieldStep {
                rule: FieldRule::HexColor,
                message,
            } => {
                if !is_hex_color_value(value) {
                    let msg =
                        validator.resolve_message(field, "hex_color", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("hex_color", msg));
                }
            }
            FieldStep {
                rule: FieldRule::MacAddress,
                message,
            } => {
                if !is_mac_address_value(value) {
                    let msg =
                        validator.resolve_message(field, "mac_address", &[], message.as_deref());
                    validator
                        .push_error(field.to_string(), ValidationError::new("mac_address", msg));
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
                rule: FieldRule::SizeNumeric(expected),
                message,
            } => {
                if !is_same_number_value(value, expected) {
                    let msg = validator.resolve_message(
                        field,
                        "size",
                        &[("size", &expected.to_string())],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("size", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Decimal { min, max },
                message,
            } => {
                if !has_decimal_places(value, min, max) {
                    let msg = validator.resolve_message(
                        field,
                        "decimal",
                        &[("min", &min.to_string()), ("max", &max.to_string())],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("decimal", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Boolean,
                message,
            } => {
                if !is_boolean_value(value) {
                    let msg = validator.resolve_message(field, "boolean", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("boolean", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Accepted,
                message,
            } => {
                if !is_accepted_value(value) {
                    let msg = validator.resolve_message(field, "accepted", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("accepted", msg));
                }
            }
            FieldStep {
                rule:
                    FieldRule::AcceptedIf {
                        other_field,
                        other_value,
                        expected_value,
                    },
                message,
            } => {
                if other_value == expected_value && !is_accepted_value(value) {
                    let msg = validator.resolve_message(
                        field,
                        "accepted_if",
                        &[("other", &other_field), ("value", &expected_value)],
                        message.as_deref(),
                    );
                    validator
                        .push_error(field.to_string(), ValidationError::new("accepted_if", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Declined,
                message,
            } => {
                if !is_declined_value(value) {
                    let msg = validator.resolve_message(field, "declined", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("declined", msg));
                }
            }
            FieldStep {
                rule:
                    FieldRule::DeclinedIf {
                        other_field,
                        other_value,
                        expected_value,
                    },
                message,
            } => {
                if other_value == expected_value && !is_declined_value(value) {
                    let msg = validator.resolve_message(
                        field,
                        "declined_if",
                        &[("other", &other_field), ("value", &expected_value)],
                        message.as_deref(),
                    );
                    validator
                        .push_error(field.to_string(), ValidationError::new("declined_if", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Alpha,
                message,
            } => {
                if !is_alpha_value(value) {
                    let msg = validator.resolve_message(field, "alpha", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("alpha", msg));
                }
            }
            FieldStep {
                rule: FieldRule::AlphaDash,
                message,
            } => {
                if !is_alpha_dash_value(value) {
                    let msg =
                        validator.resolve_message(field, "alpha_dash", &[], message.as_deref());
                    validator
                        .push_error(field.to_string(), ValidationError::new("alpha_dash", msg));
                }
            }
            FieldStep {
                rule: FieldRule::AlphaNum,
                message,
            } => {
                if !is_alpha_numeric_value(value) {
                    let msg =
                        validator.resolve_message(field, "alpha_num", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("alpha_num", msg));
                }
            }
            FieldStep {
                rule: FieldRule::AlphaNumeric,
                message,
            } => {
                if !is_alpha_numeric_value(value) {
                    let msg =
                        validator.resolve_message(field, "alpha_numeric", &[], message.as_deref());
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("alpha_numeric", msg),
                    );
                }
            }
            FieldStep {
                rule: FieldRule::Ascii,
                message,
            } => {
                if !value.is_ascii() {
                    let msg = validator.resolve_message(field, "ascii", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("ascii", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Lowercase,
                message,
            } => {
                if value != value.to_lowercase() {
                    let msg =
                        validator.resolve_message(field, "lowercase", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("lowercase", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Uppercase,
                message,
            } => {
                if value != value.to_uppercase() {
                    let msg =
                        validator.resolve_message(field, "uppercase", &[], message.as_deref());
                    validator.push_error(field.to_string(), ValidationError::new("uppercase", msg));
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
                rule: FieldRule::StartsWith(prefixes),
                message,
            } => {
                if prefixes.is_empty()
                    || !prefixes
                        .iter()
                        .any(|prefix| value.starts_with(prefix.as_str()))
                {
                    let prefixes = prefixes.join(", ");
                    let msg = validator.resolve_message(
                        field,
                        "starts_with",
                        &[("value", &prefixes)],
                        message.as_deref(),
                    );
                    validator
                        .push_error(field.to_string(), ValidationError::new("starts_with", msg));
                }
            }
            FieldStep {
                rule: FieldRule::DoesntStartWith(prefixes),
                message,
            } => {
                if prefixes
                    .iter()
                    .any(|prefix| value.starts_with(prefix.as_str()))
                {
                    let prefixes = prefixes.join(", ");
                    let msg = validator.resolve_message(
                        field,
                        "doesnt_start_with",
                        &[("value", &prefixes)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("doesnt_start_with", msg),
                    );
                }
            }
            FieldStep {
                rule: FieldRule::EndsWith(suffixes),
                message,
            } => {
                if suffixes.is_empty()
                    || !suffixes
                        .iter()
                        .any(|suffix| value.ends_with(suffix.as_str()))
                {
                    let suffixes = suffixes.join(", ");
                    let msg = validator.resolve_message(
                        field,
                        "ends_with",
                        &[("value", &suffixes)],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("ends_with", msg));
                }
            }
            FieldStep {
                rule: FieldRule::DoesntEndWith(suffixes),
                message,
            } => {
                if suffixes
                    .iter()
                    .any(|suffix| value.ends_with(suffix.as_str()))
                {
                    let suffixes = suffixes.join(", ");
                    let msg = validator.resolve_message(
                        field,
                        "doesnt_end_with",
                        &[("value", &suffixes)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("doesnt_end_with", msg),
                    );
                }
            }
            FieldStep {
                rule: FieldRule::Contains(needle),
                message,
            } => {
                if !value.contains(needle.as_str()) {
                    let msg = validator.resolve_message(
                        field,
                        "contains",
                        &[("value", &needle)],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("contains", msg));
                }
            }
            FieldStep {
                rule: FieldRule::DoesntContain(needle),
                message,
            } => {
                if value.contains(needle.as_str()) {
                    let msg = validator.resolve_message(
                        field,
                        "doesnt_contain",
                        &[("value", &needle)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("doesnt_contain", msg),
                    );
                }
            }
            FieldStep {
                rule:
                    FieldRule::MinItems(_)
                    | FieldRule::MaxItems(_)
                    | FieldRule::SizeItems(_)
                    | FieldRule::Distinct
                    | FieldRule::ContainsAll(_)
                    | FieldRule::DoesntContainAny(_),
                ..
            } => {
                return Err(Error::message(
                    "collection validation rules can only be applied with Validator::each",
                ));
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
                    FieldRule::RequiredIf {
                        other_field,
                        other_value,
                        expected_value,
                    },
                message,
            } => {
                if other_value == expected_value && value.trim().is_empty() {
                    let msg = validator.resolve_message(
                        field,
                        "required_if",
                        &[("other", &other_field), ("value", &expected_value)],
                        message.as_deref(),
                    );
                    validator
                        .push_error(field.to_string(), ValidationError::new("required_if", msg));
                }
            }
            FieldStep {
                rule:
                    FieldRule::RequiredUnless {
                        other_field,
                        other_value,
                        except_value,
                    },
                message,
            } => {
                if other_value != except_value && value.trim().is_empty() {
                    let msg = validator.resolve_message(
                        field,
                        "required_unless",
                        &[("other", &other_field), ("value", &except_value)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("required_unless", msg),
                    );
                }
            }
            FieldStep {
                rule:
                    FieldRule::RequiredIfAccepted {
                        other_field,
                        other_value,
                    },
                message,
            } => {
                if is_accepted_value(&other_value) && value.trim().is_empty() {
                    let msg = validator.resolve_message(
                        field,
                        "required_if_accepted",
                        &[("other", &other_field)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("required_if_accepted", msg),
                    );
                }
            }
            FieldStep {
                rule:
                    FieldRule::RequiredIfDeclined {
                        other_field,
                        other_value,
                    },
                message,
            } => {
                if is_declined_value(&other_value) && value.trim().is_empty() {
                    let msg = validator.resolve_message(
                        field,
                        "required_if_declined",
                        &[("other", &other_field)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("required_if_declined", msg),
                    );
                }
            }
            FieldStep {
                rule: FieldRule::Prohibited,
                message,
            } => {
                if !value.trim().is_empty() {
                    let msg =
                        validator.resolve_message(field, "prohibited", &[], message.as_deref());
                    validator
                        .push_error(field.to_string(), ValidationError::new("prohibited", msg));
                    break;
                }
            }
            FieldStep {
                rule:
                    FieldRule::ProhibitedIf {
                        other_field,
                        other_value,
                        expected_value,
                    },
                message,
            } => {
                if other_value == expected_value && !value.trim().is_empty() {
                    let msg = validator.resolve_message(
                        field,
                        "prohibited_if",
                        &[("other", &other_field), ("value", &expected_value)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("prohibited_if", msg),
                    );
                    break;
                }
            }
            FieldStep {
                rule:
                    FieldRule::ProhibitedUnless {
                        other_field,
                        other_value,
                        except_value,
                    },
                message,
            } => {
                if other_value != except_value && !value.trim().is_empty() {
                    let msg = validator.resolve_message(
                        field,
                        "prohibited_unless",
                        &[("other", &other_field), ("value", &except_value)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("prohibited_unless", msg),
                    );
                    break;
                }
            }
            FieldStep {
                rule:
                    FieldRule::ProhibitedIfAccepted {
                        other_field,
                        other_value,
                    },
                message,
            } => {
                if is_accepted_value(&other_value) && !value.trim().is_empty() {
                    let msg = validator.resolve_message(
                        field,
                        "prohibited_if_accepted",
                        &[("other", &other_field)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("prohibited_if_accepted", msg),
                    );
                    break;
                }
            }
            FieldStep {
                rule:
                    FieldRule::ProhibitedIfDeclined {
                        other_field,
                        other_value,
                    },
                message,
            } => {
                if is_declined_value(&other_value) && !value.trim().is_empty() {
                    let msg = validator.resolve_message(
                        field,
                        "prohibited_if_declined",
                        &[("other", &other_field)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("prohibited_if_declined", msg),
                    );
                    break;
                }
            }
            FieldStep {
                rule: FieldRule::Prohibits { other_fields },
                message,
            } => {
                if !value.trim().is_empty() {
                    let prohibited_fields = other_fields
                        .iter()
                        .filter_map(|(other_field, other_value)| {
                            (!other_value.trim().is_empty()).then_some(other_field.as_str())
                        })
                        .collect::<Vec<_>>();
                    if !prohibited_fields.is_empty() {
                        let other = prohibited_fields.join(", ");
                        let msg = validator.resolve_message(
                            field,
                            "prohibits",
                            &[("other", &other)],
                            message.as_deref(),
                        );
                        validator
                            .push_error(field.to_string(), ValidationError::new("prohibits", msg));
                        break;
                    }
                }
            }
            FieldStep {
                rule:
                    FieldRule::RequiredWith {
                        other_field,
                        other_value,
                    },
                message,
            } => {
                if !other_value.trim().is_empty() && value.trim().is_empty() {
                    let msg = validator.resolve_message(
                        field,
                        "required_with",
                        &[("other", &other_field)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("required_with", msg),
                    );
                }
            }
            FieldStep {
                rule: FieldRule::RequiredWithAll { other_fields },
                message,
            } => {
                if !other_fields.is_empty()
                    && other_fields
                        .iter()
                        .all(|(_, other_value)| !other_value.trim().is_empty())
                    && value.trim().is_empty()
                {
                    let other_names = other_fields
                        .iter()
                        .map(|(other_field, _)| other_field.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let msg = validator.resolve_message(
                        field,
                        "required_with_all",
                        &[("other", &other_names)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("required_with_all", msg),
                    );
                }
            }
            FieldStep {
                rule:
                    FieldRule::RequiredWithout {
                        other_field,
                        other_value,
                    },
                message,
            } => {
                if other_value.trim().is_empty() && value.trim().is_empty() {
                    let msg = validator.resolve_message(
                        field,
                        "required_without",
                        &[("other", &other_field)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("required_without", msg),
                    );
                }
            }
            FieldStep {
                rule: FieldRule::RequiredWithoutAll { other_fields },
                message,
            } => {
                if !other_fields.is_empty()
                    && other_fields
                        .iter()
                        .all(|(_, other_value)| other_value.trim().is_empty())
                    && value.trim().is_empty()
                {
                    let other_names = other_fields
                        .iter()
                        .map(|(other_field, _)| other_field.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let msg = validator.resolve_message(
                        field,
                        "required_without_all",
                        &[("other", &other_names)],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("required_without_all", msg),
                    );
                }
            }
            FieldStep {
                rule: FieldRule::RequiredKeys(_),
                ..
            } => {
                unreachable!("required_keys is handled by KeyValidator");
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
                rule: FieldRule::MinDigits(min),
                message,
            } => {
                if !has_digit_length(value, min, usize::MAX) {
                    let msg = validator.resolve_message(
                        field,
                        "min_digits",
                        &[("min", &min.to_string())],
                        message.as_deref(),
                    );
                    validator
                        .push_error(field.to_string(), ValidationError::new("min_digits", msg));
                }
            }
            FieldStep {
                rule: FieldRule::MaxDigits(max),
                message,
            } => {
                if !has_digit_length(value, 0, max) {
                    let msg = validator.resolve_message(
                        field,
                        "max_digits",
                        &[("max", &max.to_string())],
                        message.as_deref(),
                    );
                    validator
                        .push_error(field.to_string(), ValidationError::new("max_digits", msg));
                }
            }
            FieldStep {
                rule: FieldRule::DigitsBetween { min, max },
                message,
            } => {
                if !has_digit_length(value, min, max) {
                    let msg = validator.resolve_message(
                        field,
                        "digits_between",
                        &[("min", &min.to_string()), ("max", &max.to_string())],
                        message.as_deref(),
                    );
                    validator.push_error(
                        field.to_string(),
                        ValidationError::new("digits_between", msg),
                    );
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
                rule:
                    FieldRule::DateEquals {
                        other_field,
                        other_value,
                    },
                message,
            } => match compare_temporal_values(validator.app(), value, &other_value) {
                Ok(ordering) if ordering.is_eq() => {}
                Ok(_) | Err(_) => {
                    let msg = validator.resolve_message(
                        field,
                        "date_equals",
                        &[("other", &other_field)],
                        message.as_deref(),
                    );
                    validator
                        .push_error(field.to_string(), ValidationError::new("date_equals", msg));
                }
            },
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
                rule: FieldRule::MultipleOf(divisor),
                message,
            } => {
                if !is_multiple_of_value(value, divisor) {
                    let msg = validator.resolve_message(
                        field,
                        "multiple_of",
                        &[("value", &divisor.to_string())],
                        message.as_deref(),
                    );
                    validator
                        .push_error(field.to_string(), ValidationError::new("multiple_of", msg));
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
                rule: FieldRule::Gt(limit),
                message,
            } => {
                if !matches!(parse_finite_number(value), Some(num) if num > limit) {
                    let msg = validator.resolve_message(
                        field,
                        "gt",
                        &[("value", &limit.to_string())],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("gt", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Gte(limit),
                message,
            } => {
                if !matches!(parse_finite_number(value), Some(num) if num >= limit) {
                    let msg = validator.resolve_message(
                        field,
                        "gte",
                        &[("value", &limit.to_string())],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("gte", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Lt(limit),
                message,
            } => {
                if !matches!(parse_finite_number(value), Some(num) if num < limit) {
                    let msg = validator.resolve_message(
                        field,
                        "lt",
                        &[("value", &limit.to_string())],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("lt", msg));
                }
            }
            FieldStep {
                rule: FieldRule::Lte(limit),
                message,
            } => {
                if !matches!(parse_finite_number(value), Some(num) if num <= limit) {
                    let msg = validator.resolve_message(
                        field,
                        "lte",
                        &[("value", &limit.to_string())],
                        message.as_deref(),
                    );
                    validator.push_error(field.to_string(), ValidationError::new("lte", msg));
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
        }
        if bail && validator.errors.len() > errors_before {
            break;
        }
    }
    Ok(())
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
