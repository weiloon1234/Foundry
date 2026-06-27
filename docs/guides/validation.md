# Foundry Validation Rules Reference

## Usage Pattern

```rust
use foundry::prelude::*;

validator
    .field("email", &input.email)
    .bail()              // modifier: stop on first error
    .required()          // built-in: must not be empty
    .email()             // built-in: valid email format
    .unique("users", "email")  // built-in: DB check
    .apply()
    .await?;
```

All rules chain on `FieldValidator` and execute when `.apply()` is called.

Field values can be string-like or typed values that implement `ToString`, so request DTOs can keep
numeric fields as `u32`, `i64`, or similar Rust types while still using numeric validation rules.

---

## Modifiers

Modifiers change how the validation loop behaves. They are NOT rule steps — they're flags on the validator itself.

| Modifier | Effect |
|----------|--------|
| `.nullable()` | Skip all rules if the value is empty or whitespace |
| `.bail()` | Stop processing rules for this field after the first error |
| `.with_message(msg)` | Override the error message for the last added rule |

### nullable

Skip all validation when value is empty. Use for optional fields.

```rust
validator
    .field("nickname", &input.nickname)
    .nullable()
    .email()       // skipped if nickname is ""
    .min(3)        // skipped if nickname is ""
    .apply()
    .await?;
```

For `#[derive(Validate)]`, `Option<T>` fields are treated as nullable automatically when
the field does not include `required`. If an optional Rust type must still be present in
the request, add `required`:

```rust
#[derive(Deserialize, Validate)]
struct UpdateProfile {
    #[validate(required, email)]
    email: Option<String>,
}
```

Use explicit `nullable` only when an empty value should skip every other rule.

### bail

Stop on first error per field. Prevents error cascades.

```rust
validator
    .field("email", &input.email)
    .bail()
    .required()    // if this fails...
    .email()       // ...this is skipped
    .apply()
    .await?;
```

### with_message

Override the default error message for the preceding rule.

```rust
validator
    .field("age", &input.age)
    .required()
    .min_numeric(18.0)
    .with_message("You must be at least 18 years old")
    .apply()
    .await?;
```

---

## Built-in Rules

### Presence

| Rule | Code | Description |
|------|------|-------------|
| `.required()` | `required` | Value must not be empty or whitespace |
| `.filled()` | `filled` | Value must not be empty when the field is validated |
| `.required_if_accepted(field, value)` | `required_if_accepted` | Value is required when another field is accepted |
| `.required_if_declined(field, value)` | `required_if_declined` | Value is required when another field is declined |
| `.prohibited()` | `prohibited` | Value must be empty when the field is validated |
| `.prohibited_if(field, value, expected)` | `prohibited_if` | Value must be empty when another field equals `expected` |
| `.prohibited_unless(field, value, except)` | `prohibited_unless` | Value must be empty unless another field equals `except` |
| `.prohibited_if_accepted(field, value)` | `prohibited_if_accepted` | Value must be empty when another field is accepted |
| `.prohibited_if_declined(field, value)` | `prohibited_if_declined` | Value must be empty when another field is declined |
| `.prohibits([(field, value), ...])` / `#[validate(prohibits("field", ...))]` | `prohibits` | Listed fields must be empty when this value is present |

### Acceptance Rules

| Rule | Code | Description |
|------|------|-------------|
| `.accepted()` | `accepted` | Must be `yes`, `on`, `1`, or `true` |
| `.accepted_if(field, value, expected)` | `accepted_if` | Must be accepted when another field equals `expected` |
| `.declined()` | `declined` | Must be `no`, `off`, `0`, or `false` |
| `.declined_if(field, value, expected)` | `declined_if` | Must be declined when another field equals `expected` |

### String Rules

| Rule | Code | Description |
|------|------|-------------|
| `.min(n)` / `.min_length(n)` | `min` | String length must be at least `n` characters (Unicode chars, not bytes) |
| `.max(n)` / `.max_length(n)` | `max` | String length must be at most `n` characters (Unicode chars, not bytes) |
| `.size(n)` / `#[validate(size(n))]` on string fields | `size` | String length must be exactly `n` characters |
| `.alpha()` | `alpha` | Must contain only Unicode letters and marks |
| `.alpha_dash()` | `alpha_dash` | Must contain only Unicode letters, marks, numbers, ASCII dashes, and underscores |
| `.alpha_num()` | `alpha_num` | Laravel-compatible alias for Unicode alpha-numeric characters |
| `.alpha_numeric()` | `alpha_numeric` | Compatibility alias for `.alpha_num()` |
| `.ascii()` | `ascii` | Must contain only ASCII characters |
| `.lowercase()` | `lowercase` | Must be lowercase |
| `.uppercase()` | `uppercase` | Must be uppercase |
| `.digits()` | `digits` | Must contain only ASCII digits (0-9) |
| `.min_digits(n)` | `min_digits` | Must contain only ASCII digits with length at least `n` |
| `.max_digits(n)` | `max_digits` | Must contain only ASCII digits with length at most `n` |
| `.digits_between(min, max)` | `digits_between` | Must contain only ASCII digits with length between `min..=max` |
| `.starts_with(prefix)`, `.starts_with_any([...])` | `starts_with` | String must start with one of the prefixes |
| `.doesnt_start_with(prefix)`, `.doesnt_start_with_any([...])` | `doesnt_start_with` | String must not start with any forbidden prefix |
| `.ends_with(suffix)`, `.ends_with_any([...])` | `ends_with` | String must end with one of the suffixes |
| `.doesnt_end_with(suffix)`, `.doesnt_end_with_any([...])` | `doesnt_end_with` | String must not end with any forbidden suffix |
| `.contains(needle)` | `contains` | String must contain `needle` |
| `.doesnt_contain(needle)` | `doesnt_contain` | String must not contain `needle` |

### Collection Rules

| Rule | Code | Description |
|------|------|-------------|
| `.min_items(n)` | `min_items` | Collection must contain at least `n` items |
| `.max_items(n)` | `max_items` | Collection must contain at most `n` items |
| `.size_items(n)` / `#[validate(size(n))]` on `Vec<T>` / `Option<Vec<T>>` | `size` | Collection must contain exactly `n` items |
| `.distinct()` | `distinct` | Collection must not contain duplicate item values |
| `.contains_all([...])` / `#[validate(contains(...))]` on `Vec<T>` / `Option<Vec<T>>` | `contains` | Collection must contain all required values |
| `.doesnt_contain_any([...])` / `#[validate(doesnt_contain(...))]` on `Vec<T>` / `Option<Vec<T>>` | `doesnt_contain` | Collection must not contain any forbidden values |

### Map/Object Rules

| Rule | Code | Description |
|------|------|-------------|
| `.keys(...).required_keys([...])` / `#[validate(required_keys("timezone", "locale"))]` on `HashMap<String, T>`, `BTreeMap<String, T>`, or `serde_json::Value` object fields | `required_keys` | Object/map must contain the listed keys; `null` values still count as present |

### Numeric Rules

| Rule | Code | Description |
|------|------|-------------|
| `.numeric()` | `numeric` | Must be a numeric string (digits, optional `.`, `-`, `+`) |
| `.integer()` | `integer` | Must parse as a valid integer (`i64`) |
| `.decimal(min, max)` | `decimal` | Must be numeric text with `min..=max` decimal places |
| `.min_numeric(n)` | `min_numeric` | Parsed number must be at least `n` |
| `.max_numeric(n)` | `max_numeric` | Parsed number must be at most `n` |
| `.size_numeric(n)` / `#[validate(size(n))]` on numeric fields | `size` | Parsed number must equal `n` |
| `.multiple_of(n)` | `multiple_of` | Parsed number must be a multiple of `n` |
| `.between(min, max)` | `between` | Parsed number must be between `min` and `max` (inclusive) |
| `.gt(n)` | `gt` | Parsed number must be greater than `n` |
| `.gte(n)` | `gte` | Parsed number must be greater than or equal to `n` |
| `.lt(n)` | `lt` | Parsed number must be less than `n` |
| `.lte(n)` | `lte` | Parsed number must be less than or equal to `n` |

### Format Rules

| Rule | Code | Description |
|------|------|-------------|
| `.boolean()` | `boolean` | Must be `true`, `false`, `1`, or `0` |
| `.email()` | `email` | Must be a valid HTML-style email address; IDN domains and whole-domain IP literals such as `user@[127.0.0.1]` are allowed |
| `.url()` | `url` | Must be a valid URL without literal whitespace; encode spaces as `%20` |
| `.uuid()` / `.uuid_version(n)` | `uuid` | Must be a valid UUID, optionally constrained to version `n` |
| `.ulid()` | `ulid` | Must be a valid ULID |
| `.hex_color()` | `hex_color` | Must be a valid hexadecimal color |
| `.mac_address()` | `mac_address` | Must be a valid MAC address |
| `.regex(pattern)` | `regex` | Must match the given Rust regex pattern |
| `.not_regex(pattern)` | `not_regex` | Must not match the given Rust regex pattern |
| `.json()` | `json` | String values must contain valid JSON; `serde_json::Value` fields are already parsed JSON and generated clients treat the rule as server-only metadata |
| `.timezone()` | `timezone` | Must be `UTC`, an exact `chrono_tz` timezone name such as `Asia/Kuala_Lumpur`, or a fixed offset like `+08:00` / `+0800` |
| `.date()` | `date` | Must be a valid chrono date; unsigned years use `YYYY-MM-DD`, expanded years require a sign such as `+10000-01-01` |
| `.time()` | `time` | Must be a valid `HH:MM:SS` time |
| `.datetime()` | `datetime` | Must be a valid offset-aware datetime, or an offset-less datetime interpreted in the app timezone |
| `.local_datetime()` | `local_datetime` | Must be a valid timezone-less local datetime |

### IP Address Rules

| Rule | Code | Description |
|------|------|-------------|
| `.ip()` | `ip` | Must be a valid IP address (IPv4 or IPv6) |
| `.ipv4()` | `ipv4` | Must be a valid IPv4 address only |
| `.ipv6()` | `ipv6` | Must be a valid IPv6 address only |

### List Rules

| Rule | Code | Description |
|------|------|-------------|
| `.in_list([...])` | `in_list` | Value must be in the given list |
| `.not_in([...])` | `not_in` | Value must NOT be in the given list |

### Comparison Rules

| Rule | Code | Description |
|------|------|-------------|
| `.required_if(field, value, expected)` | `required_if` | Value is required when another field equals `expected` |
| `.required_unless(field, value, except)` | `required_unless` | Value is required unless another field equals `except` |
| `.required_with(field, value)` | `required_with` | Value is required when another field is present |
| `.required_with_all([(field, value), ...])` | `required_with_all` | Value is required when all listed fields are present |
| `.required_without(field, value)` | `required_without` | Value is required when another field is not present |
| `.required_without_all([(field, value), ...])` | `required_without_all` | Value is required when none of the listed fields are present |
| `.confirmed(field, value)` / `#[validate(confirmed)]` | `confirmed` | Value must match another field; derive defaults to `<field>_confirmation` and accepts `confirmed("field")` for custom siblings |
| `.same(field, value)` | `same` | Value must match the given value |
| `.different(field, value)` | `different` | Value must differ from the given value |
| `.before(field, value)` | `before` | Value must be before the given temporal value |
| `.before_or_equal(field, value)` | `before_or_equal` | Value must be before or equal to the given temporal value |
| `.after(field, value)` | `after` | Value must be after the given temporal value |
| `.after_or_equal(field, value)` | `after_or_equal` | Value must be after or equal to the given temporal value |
| `.date_equals(field, value)` | `date_equals` | Value must equal the given temporal value |

Temporal comparison rules support `foundry::DateTime`, `foundry::LocalDateTime`, `foundry::Date`, and `foundry::Time` string formats. Offset-less `.datetime()` values are interpreted in the configured app timezone.

### Enum Rules

| Rule | Code | Description |
|------|------|-------------|
| `.app_enum::<E>()` | `app_enum` | Value must be a canonical key or declared alias in the given `FoundryAppEnum` type |

```rust
validator
    .field("status", &input.status)
    .required()
    .app_enum::<OrderStatus>()
    .apply()
    .await?;
```

### Database Rules (async)

These rules query the database. They require an active database connection via `AppContext`.

| Rule | Code | Description |
|------|------|-------------|
| `.unique(table, column)` | `unique` | Value must NOT exist in the given table/column |
| `.exists(table, column)` | `exists` | Value MUST exist in the given table/column |

```rust
validator
    .field("email", &input.email)
    .unique("users", "email")
    .apply()
    .await?;

validator
    .field("country_id", &input.country_id)
    .exists("countries", "id")
    .apply()
    .await?;
```

---

## Array / Collection Validation

Use `.min_items()` / `.max_items()` to validate collection length, and `.each()`
to validate each item in a collection. Item field rules are available on the
`EachValidator`. Derived validators support both `Vec<T>` and `Option<Vec<T>>`;
absent optional collections are skipped unless you add `required`, `filled`, or
another presence rule. A present-but-empty `Option<Vec<T>>` still runs collection
rules such as `min_items`, `size`, and `contains`.
Generated TypeScript keeps the backend `size` error code for collection
`size(...)`, and adds `params.kind = "array"` so `validateForm()` can enforce an
exact item count without confusing collection size with string length.

```rust
validator
    .each("tags", &input.tags)
    .bail()
    .min_items(1)
    .max_items(10)
    .distinct()
    .required()
    .min(2)
    .max(50)
    .apply()
    .await?;
```

Errors are reported with indexed field names (e.g. `tags[0]`, `tags[1]`).
Custom attributes and custom messages registered for the base field, such as
`tags`, also apply to indexed item errors like `tags[0]`. For nested indexed
paths, Foundry removes index segments before the base fallback, so metadata for
`addresses.streetName` also applies to `addresses[0].streetName`.

Nested request DTOs are explicit. Add `#[validate(nested)]` to validate a child
DTO field, or `#[validate(each(nested))]` on `Vec<ChildDto>` /
`Option<Vec<ChildDto>>` to validate every child item. Child errors are reported with parent prefixes like
`primaryAddress.streetName` and `previousAddresses[0].streetName`, while the
child DTO still owns its own custom messages and attributes.
For multipart requests, nested DTO fields are parsed from JSON text parts named
after the field; use normal JSON request bodies for deeply nested payloads when
possible.

```rust
#[derive(Deserialize, ApiSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct AddressInput {
    #[validate(required)]
    pub street_name: String,
}

#[derive(Deserialize, ApiSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct ProfileInput {
    #[validate(nested)]
    pub primary_address: AddressInput,

    #[validate(min_items(1), each(nested))]
    pub previous_addresses: Vec<AddressInput>,
}
```

---

## Validator Methods

The `Validator` struct provides these methods for controlling validation behavior:

| Method | Description |
|--------|-------------|
| `.locale(locale)` | Set locale for error message translation |
| `.set_locale(locale)` | Set locale (mutable version) |
| `.custom_message(field, code, message)` | Override error message for a specific field + rule |
| `.custom_attribute(field, name)` | Override display name for a field in error messages |
| `.add_error(field, code, params)` | Manually add a validation error |
| `.finish()` | Return `Ok(())` or `Err(ValidationErrors)` |

---

## Custom Rules

### Define

```rust
use async_trait::async_trait;
use foundry::validation::{RuleContext, ValidationError, ValidationRule};

pub struct MobileRule;

#[async_trait]
impl ValidationRule for MobileRule {
    async fn validate(
        &self,
        context: &RuleContext,
        value: &str,
    ) -> std::result::Result<(), ValidationError> {
        // context.app() gives AppContext — access database, config, etc.
        if value.starts_with('+') && value[1..].chars().all(|c| c.is_ascii_digit()) {
            Ok(())
        } else {
            Err(ValidationError::new("mobile", "invalid mobile number"))
        }
    }
}
```

### Register

```rust
App::builder()
    .register_validation_rule("mobile", MobileRule)
    .run_http()?;
```

### Use

```rust
validator
    .field("phone", &input.phone)
    .required()
    .rule(ValidationRuleId::new("mobile"))
    .apply()
    .await?;
```

Derived validators can reference either a literal rule name or a typed `ValidationRuleId`
constant. Prefer constants in applications so the registered rule and request DTO share
one source:

```rust
pub const MOBILE: ValidationRuleId = ValidationRuleId::new("mobile");

#[derive(Deserialize, Validate)]
pub struct SendSms {
    #[validate(required, rule(MOBILE))]
    pub phone: String,
}
```

Generated TypeScript and OpenAPI metadata expose custom rule ids as server-only
validation rules, so `rule(MOBILE)` appears as `code: "mobile"` instead of a
generic wrapper rule. The backend remains responsible for executing the custom
rule. `types:export` writes `ValidationRuleManifest.ts` with the registered
custom rule ids and `serverOnly: true` metadata, and app-backed OpenAPI specs
publish the same map as `x-foundry-validation-rules`, so frontend forms and
OpenAPI tooling can discover backend-owned custom checks without copying rule
names by hand. Form builders can use `validationRuleId(...)`,
`validationRuleIds()`,
`validationRuleIdCount()`, `validationRuleHasIds()`,
`validationRuleFirstId()`, `validationRuleFirstIdOrNull()`,
`validationRuleNames()`,
`validationRuleEntries()`, `isValidationRuleId()`,
`validationRuleIdOrNull()`, `validationRuleNameOrNull()`,
`validationRuleManifestEntryOrNull()`, `validationRuleManifestEntryById()`,
`validationRuleManifestEntryByIdOrNull()`,
`validationRuleIsRegistered()`,
`validationRuleIdForNameOrNull()`, `validationRuleIdIsServerOnly()`,
`validationRuleIsServerOnlyOrNull()`, `validationRuleFirstEntryOrNull()`,
`validationRuleFirstNameOrNull()`, `serverOnlyValidationRuleIds()`,
`serverOnlyValidationRuleIdCount()`, `serverOnlyValidationRuleHasIds()`,
`serverOnlyValidationRuleFirstId()`, `serverOnlyValidationRuleFirstIdOrNull()`,
`serverOnlyValidationRuleNames()`,
`serverOnlyValidationRuleNameCount()`, `serverOnlyValidationRuleHasNames()`,
`serverOnlyValidationRuleFirstName()`,
`serverOnlyValidationRuleFirstNameOrNull()`, `serverOnlyValidationRules()`,
`serverOnlyValidationRuleCount()`, `serverOnlyValidationRuleHasEntries()`,
`serverOnlyValidationRuleFirstEntry()`, and
`serverOnlyValidationRuleFirstEntryOrNull()` to list, filter, validate,
summarize, or look up registered backend-only custom rules directly from
generated TypeScript, including runtime rule ids read from endpoint validation
metadata and first selector results that should be modeled as explicit `null`.
Custom rule ids must use non-empty dotted segments and avoid
camelCase-normalized segment collisions so `ValidationRuleManifest.ts`,
`ValidationRuleIds`, and app-backed OpenAPI `x-foundry-validation-rules`
represent the same backend-owned rule set.
Generated validation-rule manifest constants are frozen at runtime, while
selector helpers return cloned entries for local form-builder or docs
annotations.
Use `validationRuleIdOrNull(value)` to parse a runtime rule id, and
`validationRuleIdForNameOrNull(name)` or `validationRuleIsServerOnlyOrNull(name)`
when a runtime rule name should drive nullable metadata reads.
App-backed `types:export` and built-in OpenAPI spec generation check explicit
`rule(...)` and `TsValidationRule::custom(...)` metadata against the runtime
rule registry, so custom rule metadata must point at a registered custom rule
id. Manual custom metadata must also keep the rule code and `params.rule` equal;
register the rule before exporting frontend types or booting OpenAPI docs, or
remove stale metadata.

### Translating Custom Rule Messages

Custom rule error messages go through the same i18n pipeline as built-in rules. The `code` field in `ValidationError` is used as the translation key. Add entries under `validation` in your locale files:

```json
// locales/en/validation.json
{
    "mobile": "The :attribute field must be a valid mobile number."
}

// locales/zh/validation.json
{
    "mobile": ":attribute 必须是有效的手机号码。"
}
```

The lookup priority is:
1. Inline `.with_message()` override
2. Validator-level `custom_message(field, code, msg)` (exact field, then unindexed base field for `each(...)` item errors)
3. i18n `validation.custom.{field}.{code}` (exact field, then unindexed base field for `each(...)` item errors)
4. i18n `validation.{code}` (matched by the error's `code` field)
5. Hardcoded fallback

---

## File Upload Validation

Utility functions for validating uploaded files (not chained on `FieldValidator`):

| Function | Description |
|----------|-------------|
| `is_image(file)` | Check if file is an image (magic bytes) |
| `check_max_size(file, max_kb)` | Check file size <= max_kb KB |
| `get_image_dimensions(file)` | Get image (width, height) |
| `check_allowed_mimes(file, allowed)` | Check MIME type against allowed list |
| `check_allowed_extensions(file, allowed)` | Check extension against allowed list |

```rust
use foundry::validation::file_rules;

if !file_rules::check_max_size(&file, 2048) {
    // file exceeds 2MB
}

if !file_rules::is_image(&file).await? {
    // not a valid image
}
```

When file rules are declared with `#[derive(Validate)]`, generated TypeScript
route helpers prevalidate the metadata-only rules that browser `File` objects can
prove (`max_file_size`, `allowed_extensions`). Content-sniffing rules such as
`image`, `allowed_mimes`, `max_dimensions`, and `min_dimensions` stay marked as
server-only metadata because the backend verifies bytes and image headers.
File rules can be declared on `UploadedFile`, `Option<UploadedFile>`,
`Vec<UploadedFile>`, and `Option<Vec<UploadedFile>>`. For multi-file fields,
collection rules such as `filled`, `min_items`, `max_items`, and `size` validate
the number of files on the base field, while file rules run against every file
and report indexed errors such as `photos[0]`. Absent optional multi-file fields
skip file validation unless a presence rule requires the collection. OpenAPI
schemas mirror that split by keeping file-count constraints on the array schema
and emitting per-file `x-foundry-*` upload metadata on the array `items` schema.
Generated TypeScript route helpers also apply browser-checkable multi-file rules
per file, so client-side prevalidation reports the same indexed paths as the
backend for size and extension failures.

---

## Request Validation (HTTP handlers)

### Derive-based (recommended)

Use `#[derive(Validate)]` to generate validation from field attributes. Combine with `#[derive(ApiSchema)]` to also generate OpenAPI schema from the same struct:

```rust
#[derive(Deserialize, ApiSchema, Validate)]
#[validate(
    messages(email(unique = "This email is already registered")),
    attributes(email = "email address")
)]
pub struct CreateUser {
    #[validate(required, email, unique("users", "email"))]
    pub email: String,

    #[validate(required, min(8), confirmed)]
    pub password: String,

    #[validate(required)]
    pub password_confirmation: String,

    #[validate(required, app_enum(UserStatus))]
    pub status: UserStatus,   // AppEnum validated + OpenAPI schema auto-resolved
}
```

Use `make:request --name CreateUserRequest` to start from a compiling DTO that
already derives `serde::Deserialize`, `foundry::ts_rs::TS`, `ApiSchema`, and
`Validate`. Edit the generated fields and validation attributes, then use the
type with `JsonValidated<T>` or `Validated<T>` in a route.

If the DTO uses serde `rename` or `rename_all`, derived validation errors,
custom message/attribute keys, generated TypeScript validation metadata, and
multipart field matching use the serde wire field names. Validation attribute
references to sibling fields still use Rust field identifiers so the macro can
type-check `self.<field>` access.

For request-specific checks that still need generated field metadata, keep the
field rules on the DTO and add an async after-hook:

```rust
#[derive(Deserialize, ApiSchema, Validate)]
#[validate(after(validate_signup))]
pub struct SignupRequest {
    #[validate(required, email)]
    pub email: String,

    #[validate(required)]
    pub country_iso2: String,

    pub contact_number: Option<String>,
}

async fn validate_signup(req: &SignupRequest, validator: &mut Validator) -> Result<()> {
    if !phone_matches_country(&req.country_iso2, req.contact_number.as_deref()) {
        validator.add_error("contact_number", "phone_invalid_for_country", &[]);
    }

    Ok(())
}
```

Generated TypeScript and OpenAPI schemas expose after-hooks as schema-level,
server-only validation metadata, for example
`{ code: "after", params: { hook: "validate_signup" }, serverOnly: true }`.

`#[derive(ApiSchema)]` reads `#[validate(...)]` attributes and converts them to JSON Schema constraints:

| Validate attribute | OpenAPI Schema |
|-------------------|----------------|
| `required` | Added to `"required"` array |
| `nullable` | Added to `"x-foundry-validation"` metadata; this does not make a non-`Option` Rust field deserialize from JSON `null` |
| `bail` | Added to `"x-foundry-validation"` metadata so generated validators stop after the first field error |
| `email` | `"format": "email"` |
| `url` | `"format": "uri"` |
| `uuid` / `uuid(n)` | `"format": "uuid"`; version-constrained rules also emit a compatible UUID pattern when the version is a supported literal |
| `ulid` | `"pattern": "^[0-7][0-9A-HJKMNP-TV-Za-hjkmnp-tv-z]{25}$"` |
| `hex_color` | `"pattern": "^#(?:[0-9A-Fa-f]{3}\|[0-9A-Fa-f]{4}\|[0-9A-Fa-f]{6}\|[0-9A-Fa-f]{8})$"` |
| `mac_address` | `"pattern": "^(?:(?:[0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}\|(?:[0-9A-Fa-f]{2}-){5}[0-9A-Fa-f]{2})$"` |
| `boolean` | String enum `["true", "false", "1", "0"]` unless the field schema is already boolean |
| `accepted` | String enum `["yes", "on", "1", "true"]`, or boolean enum `[true]` |
| `declined` | String enum `["no", "off", "0", "false"]`, or boolean enum `[false]` |
| `accepted_if(...)`, `declined_if(...)`, `required_if(...)`, `required_unless(...)`, `required_if_accepted(...)`, `required_if_declined(...)`, `required_with(...)`, `required_with_all(...)`, `required_without(...)`, `required_without_all(...)`, `prohibited`, `prohibited_if(...)`, `prohibited_unless(...)`, `prohibited_if_accepted(...)`, `prohibited_if_declined(...)`, `prohibits(...)` | Added to `"x-foundry-validation"` metadata; no unconditional field-local schema constraint |
| `regex("pattern")` | `"pattern": "pattern"` |
| `not_regex("pattern")` | `"not": { "pattern": "pattern" }` |
| `starts_with("a", "b")` | Escaped prefix `"pattern"` or `anyOf` patterns when multiple values are supplied |
| `doesnt_start_with("a", "b")` | `"not": { "pattern": "^a" }` or `"not": { "anyOf": [...] }` when multiple values are supplied |
| `ends_with("a", "b")` | Escaped suffix `"pattern"` or `anyOf` patterns when multiple values are supplied |
| `doesnt_end_with("a", "b")` | `"not": { "pattern": "a$" }` or `"not": { "anyOf": [...] }` when multiple values are supplied |
| `contains("needle")` on strings | Escaped substring `"pattern"` |
| `doesnt_contain("needle")` on strings | Escaped substring `"not.pattern"` |
| `contains("a", "b")` on arrays | `"contains": { "const": ... }` constraints, grouped under `allOf` when multiple values are supplied |
| `doesnt_contain("a", "b")` on arrays | `"not": { "contains": { "enum": [...] } }` |
| `required_keys("a", "b")` on maps/objects | Adds object `"required": ["a", "b"]` and matching `"x-foundry-validation"` metadata |
| `size(N)` | Exact string length, array item count, or numeric value constraints depending on field schema type |
| `filled` | String schemas get `"pattern": "\\S"`; array schemas get `"minItems": 1` |
| `alpha` | `"pattern": "^[\\p{L}\\p{M}]*$"` |
| `alpha_num`, `alpha_numeric` | `"pattern": "^[\\p{L}\\p{M}\\p{N}]*$"` |
| `alpha_dash` | `"pattern": "^[\\p{L}\\p{M}\\p{N}_-]*$"` |
| `ascii` | `"pattern": "^[\\x00-\\x7F]*$"` |
| `lowercase` | `"pattern": "^[^\\p{Lu}]*$"` |
| `uppercase` | `"pattern": "^[^\\p{Ll}]*$"` |
| `numeric` | String schemas get a finite decimal-number `"pattern"` |
| `integer` | String schemas get an integer `"pattern"`; number schemas get `"multipleOf": 1`; schemas also include `"x-foundry-integer-format": "i64"` |
| `decimal(min, max)` | `"pattern": "^[+-]?(?:[0-9]+\\.[0-9]{min,max}\|\\.[0-9]{min,max})$"` |
| `digits` | `"pattern": "^[0-9]*$"` |
| `min_digits(N)` | `"pattern": "^[0-9]*$"`, `"minLength": N` |
| `max_digits(N)` | `"pattern": "^[0-9]*$"`, `"maxLength": N` |
| `digits_between(MIN, MAX)` | `"pattern": "^[0-9]*$"`, `"minLength": MIN`, `"maxLength": MAX` |
| `date` | `"format": "date"` |
| `time` | `"format": "time"` |
| `datetime` / `local_datetime` | `"format": "date-time"` |
| `timezone` | Custom `"format": "timezone"` marker |
| `confirmed`, `same(...)`, `different(...)`, `before(...)`, `before_or_equal(...)`, `after(...)`, `after_or_equal(...)`, `date_equals(...)` | Added to `"x-foundry-validation"` metadata; no unconditional field-local schema constraint |
| `ip` | Custom `"format": "ip"` marker |
| `ipv4` / `ipv6` | `"format": "ipv4"` / `"format": "ipv6"` |
| `json` | String schemas get custom `"format": "json-string"` marker; parsed JSON schemas get server-only `"x-foundry-validation"` metadata |
| `min(N)`, `min_length(N)` | `"minLength": N` |
| `max(N)`, `max_length(N)` | `"maxLength": N` |
| `min_items(N)` | `"minItems": N` |
| `max_items(N)` | `"maxItems": N` |
| `distinct` | `"uniqueItems": true` |
| `max_file_size(KB)` | `"x-foundry-max-file-size-kb": KB` on upload schemas |
| `allowed_extensions("jpg", "png")` | `"x-foundry-allowed-extensions": ["jpg", "png"]` on upload schemas |
| `image` | Adds `"image"` to `"x-foundry-server-only-validation"` on upload schemas |
| `allowed_mimes("image/png")` | `"x-foundry-allowed-mimes": ["image/png"]` and server-only marker on upload schemas |
| `max_dimensions(W, H)` | `"x-foundry-max-dimensions": {"width": W, "height": H}` and server-only marker on upload schemas |
| `min_dimensions(W, H)` | `"x-foundry-min-dimensions": {"width": W, "height": H}` and server-only marker on upload schemas |
| `nested` | Added to `"x-foundry-validation"` metadata on the child object schema |
| `min_numeric(N)` | `"minimum": N` |
| `max_numeric(N)` | `"maximum": N` |
| `multiple_of(N)` | `"multipleOf": N` |
| `between(MIN, MAX)` | `"minimum": MIN`, `"maximum": MAX` |
| `gt(N)` | `"exclusiveMinimum": N` |
| `gte(N)` | `"minimum": N` |
| `lt(N)` | `"exclusiveMaximum": N` |
| `lte(N)` | `"maximum": N` |
| `in_list("a", "b")` | `"enum": ["a", "b"]`; numeric fields keep numeric enum values |
| `not_in("a", "b")` | `"not": { "enum": ["a", "b"] }`; numeric fields keep numeric enum values |
| `each(min(N), max(N), ...)` on `Vec<T>` / `Option<Vec<T>>` | Applies supported constraints to the array `"items"` schema, including `each(nested)` metadata for child DTO items |
| `app_enum(UserStatus)` | Accepted values from `UserStatus` when the field schema does not already expose an enum; raw string fields include aliases, while raw numeric fields use numeric canonical values |
| `unique(table, column)`, `exists(table, column)` | Added to `"x-foundry-validation"` metadata with `"serverOnly": true` |

Unknown struct-level or field-level `#[validate(...)]` names now fail
`#[derive(ApiSchema)]` compilation instead of being silently ignored. Malformed
struct-level `messages(...)` / `attributes(...)` metadata is also rejected even
when a DTO derives `ApiSchema` without `Validate`, and blank or duplicate
message/attribute entries fail compilation instead of generating empty copy or
silently overwriting an earlier entry. For `#[derive(Validate)]`, custom
messages must target fields with validation rules, and message rule names must
match reachable field rules unless the struct has an `after(...)` hook that can
emit server-only rule codes; after-hook messages and attributes may target any
request contract field, even when that field has no local validation rule.
Custom attribute labels must target fields with validation rules or after-hook
contract fields, matching generated TypeScript validation metadata.
No-argument rules reject stray positional
arguments such as `required("message")`. Use `required(message = "...")` for
per-rule message overrides; blank or repeated `message = "..."` overrides are
rejected. Single-argument rules such as `min(...)`, `regex(...)`, and
`max_file_size(...)` also reject extra positional values. File validation rules,
`app_enum(...)`, and `each(...)` also validate their arity; `each()` is rejected
because it does not describe any nested item rule. This keeps OpenAPI schemas,
generated TypeScript validation metadata, and backend validation attributes
aligned. `min_length(...)` and `max_length(...)` are accepted aliases for
`min(...)` and `max(...)`; generated backend errors, TypeScript validation
metadata, and custom message rule keys use the canonical `min` / `max` codes.
`messages(field(min_length = "..."))` / `messages(field(max_length = "..."))`
are accepted and normalized to `min` / `max`.

Generated schema property names and simple enum values follow serde wire names,
including `#[serde(rename = "...")]` and supported `#[serde(rename_all = "...")]`
rules. That keeps OpenAPI, generated TypeScript, and JSON request/response bodies
on the same naming contract. Public DTO fields must resolve to unique JSON names;
Foundry rejects duplicate serde wire names instead of letting validation errors,
generated TypeScript, or OpenAPI property maps disagree about the owner of a
JSON key. Plain contract enum variants must also resolve to unique JSON names so
generated unions and OpenAPI enum values stay one-to-one with backend variants.
`#[serde(alias = "...")]` is rejected on public contract fields and enum variants
because it makes the backend accept alternate input names that generated
TypeScript and OpenAPI cannot honestly advertise as the canonical contract.
Custom field codecs such as `#[serde(with = "...")]`,
`#[serde(serialize_with = "...")]`, and `#[serde(deserialize_with = "...")]`
are also rejected on public contract fields because generated validation
metadata, TypeScript, and OpenAPI cannot infer the hidden wire shape.
TypeScript-only renames such as `#[ts(rename = "...")]` and
`#[ts(rename_all = "...")]` are rejected on public contracts; use serde
`rename` / `rename_all` so validation metadata, JSON, OpenAPI, and TypeScript
share the same field names.
`#[serde(flatten)]` fields are expanded into parent-level schema properties so
flattened DTOs document the same shape they serialize. Flattened `Option<T>`
fields are rejected because generated
TypeScript cannot represent them safely; flatten a normal child DTO and make the
child fields optional instead. Flattened fields are also rejected on
`#[serde(deny_unknown_fields)]` DTOs because serde cannot safely combine
flattened fields with strict unknown-field rejection. Derived OpenAPI schemas
also fail fast when flattened child fields collide with parent fields or with
another flattened child after serde renaming; rename the fields or split the DTO
so every flattened property is unique. Non-`Option` fields with
`#[serde(default)]` are not OpenAPI-required unless they also have
`#[validate(required)]`; add
`#[ts(optional, as = "Option<_>")]` so generated TypeScript accepts the same
omitted input as serde. Validation-only `message = "..."` overrides are ignored
by `ApiSchema` while preserving the underlying schema constraint. Directional
skips such as `#[serde(skip_serializing)]` and `#[serde(skip_deserializing)]` are
rejected for generated contracts because one DTO cannot safely describe both
request and response shapes; split the DTOs or fully skip never-public fields.
For `#[derive(Validate)]`, fields skipped during request deserialization with
`#[serde(skip)]` or `#[serde(skip_deserializing)]` are not request fields: they
are omitted from strict generated validation metadata, ignored by generated
multipart extraction, initialized from serde-style defaults, and cannot carry
field-level `#[validate(...)]` rules.
`#[serde(deny_unknown_fields)]` is emitted as `additionalProperties: false`, and
derived TypeScript validation metadata sets `denyUnknownFields: true` for DTOs
that also derive `Validate`, so OpenAPI consumers and route helpers see the same
unknown-field rejection that serde applies.
Upload-specific metadata that is not standard JSON Schema is emitted through
`x-foundry-*` vendor extensions so generated clients and docs can reuse the
same backend-owned file contract.

Fields marked `#[serde(flatten)]` cannot carry `#[validate(...)]` rules today
because the generated validation metadata and error paths would target the Rust
wrapper field instead of the flattened JSON keys. Move those rules to explicit
parent fields or avoid flattening DTOs that need nested validation.
Validation-only rules that depend on sibling fields, temporal comparisons,
database state, custom rule registries, or struct-level after-hooks are emitted in the
`"x-foundry-validation"` vendor extension using the same `code`, `params`,
`values`, and `serverOnly` shape as generated TypeScript route helpers. Field
references inside this OpenAPI metadata use the same serde wire names as schema
properties, including `rename` and `rename_all`.
OpenAPI request schemas and expanded query operations also include
`x-foundry-validation-field-value-kinds` when cross-field validation may need
non-scalar sibling semantics, using `{ field, kind }` rows with `array`, `map`,
`json`, `file`, or `nested` kinds.
For app-backed OpenAPI docs, custom rule metadata is checked against the runtime
rule registry before the spec is cached for the observability route.
For GET/HEAD query DTOs, OpenAPI expands fields into query parameters; schema-level
validation metadata such as `#[validate(after(...))]` and non-scalar field value
kind rows are kept on the operation while field-local metadata stays on the
individual parameter schemas.
When multiple string pattern rules apply to one field, Foundry emits them under
`allOf` so `regex(...)`, `starts_with(...)`, `ends_with(...)`, `contains(...)`,
`alpha`, `alpha_num`, `alpha_numeric`, `alpha_dash`, `ulid`, `hex_color`,
`mac_address`, `decimal(...)`, `ascii`, `digits`, `min_digits(...)`,
`max_digits(...)`, and `digits_between(...)` do not overwrite each other.
Negative pattern constraints such as `not_regex(...)`, `doesnt_start_with(...)`,
and `doesnt_end_with(...)` are emitted as JSON Schema `not` constraints; multiple
literal prefixes or suffixes are grouped under `anyOf`.
Derived `regex(...)` / `not_regex(...)` rules that Foundry cannot safely
translate to JavaScript are emitted as server-only `x-foundry-validation`
metadata instead of JSON Schema `pattern` constraints.

`decimal(min, max)` validates the textual decimal places supplied in the request. Prefer `String` or an exact decimal-text input type for request DTO fields that use this rule; binary floating-point fields cannot preserve whether the client sent `12.30` or `12.3`.

### Manual (full control)

For complex validation logic that can't be expressed in attributes:

```rust
#[derive(Deserialize)]
pub struct CreateUser {
    pub email: String,
    pub password: String,
    pub password_confirmation: String,
}

#[async_trait]
impl RequestValidator for CreateUser {
    async fn validate(&self, validator: &mut Validator) -> Result<()> {
        validator
            .field("email", &self.email)
            .bail()
            .required()
            .email()
            .unique("users", "email")
            .apply()
            .await?;

        validator
            .field("password", &self.password)
            .bail()
            .required()
            .min(8)
            .confirmed("password_confirmation", &self.password_confirmation)
            .apply()
            .await?;

        Ok(())
    }
}
```

Manual validators can still publish backend-owned frontend and OpenAPI metadata
when the DTO also exports TypeScript:

```rust
use foundry::prelude::*;

impl TsValidationSchemaProvider for CreateUser {
    fn ts_validation_schema() -> TsValidationSchema {
        TsValidationSchema::new()
            .field(
                "email",
                [
                    TsValidationRule::required(),
                    TsValidationRule::email(),
                    TsValidationRule::unique("users", "email"),
                ],
            )
            .field(
                "password",
                [
                    TsValidationRule::required(),
                    TsValidationRule::min(8),
                    TsValidationRule::confirmed("password_confirmation"),
                ],
            )
    }
}

foundry::inventory::submit! {
    TsValidation {
        name: "CreateUser",
        schema_fn: || <CreateUser as TsValidationSchemaProvider>::ts_validation_schema(),
    }
}
```

For manual collection metadata, prefer `TsValidationRule::size_items(n)`. It
exports the same canonical `size` rule with `kind = "array"` that
`#[validate(size(n))]` emits for `Vec<T>` fields, so client-side validation,
custom messages, and backend error codes stay aligned. Legacy manual
`TsValidationRule::new("size_items").param("size", n)` entries are accepted at
registration and normalized before export. Manual `size` metadata with
`kind = "array"` must use a non-negative integer `size`, matching backend
collection length semantics.
For collection membership metadata, use `TsValidationRule::contains_all([...])`
or `TsValidationRule::doesnt_contain_any([...])`; both emit the canonical
`contains` / `doesnt_contain` codes plus the same `values` and `value` summary
metadata generated by `#[validate(contains(...))]` on collection fields.
For manual string prefix/suffix/content metadata, use scalar helpers such as
`TsValidationRule::starts_with_value(...)`,
`doesnt_start_with_value(...)`, `ends_with_value(...)`, and
`doesnt_end_with_value(...)`, `contains_value(...)`, and
`doesnt_contain_value(...)` when the runtime rule checks one value. Use
`starts_with_any(...)`, `doesnt_start_with_any(...)`, `ends_with_any(...)`,
`doesnt_end_with_any(...)`, or their multi-value counterparts for multi-value
metadata; all helpers emit canonical rule codes used by derived metadata.
For sibling-field metadata, prefer helper constructors such as
`TsValidationRule::required_if("enabled", true)`,
`TsValidationRule::required_with("guest_reason")`,
`TsValidationRule::confirmed("password_confirmation")`, and
`TsValidationRule::before_or_equal("deadline")`; they emit the same `other` and
`value` params that derive-generated metadata uses.
When a manual sibling-field rule references a non-scalar field, also register the
referenced field's value kind so generated TypeScript compares the same presence
or string value as the backend:

```rust
TsValidationSchema::new()
    .field_value_kind("attachments", TsValidationFieldValueKind::Array)
    .field_value_kind("settings", TsValidationFieldValueKind::Map)
    .field("note", [TsValidationRule::required_with("attachments")])
```

Use `Array`, `Map`, `Json`, `File`, or `Nested` for collection/object/upload/DTO
siblings; derived `Validate` schemas emit this metadata automatically for
non-scalar contract fields, including fields that have no direct validation rule.
For presence, scalar, numeric, list, and file metadata, prefer typed
constructors such as `TsValidationRule::required()`,
`TsValidationRule::email()`, `TsValidationRule::min(8)`,
`TsValidationRule::min_length(8)`, `TsValidationRule::decimal(2, 4)`,
`TsValidationRule::size_numeric(10)`,
`TsValidationRule::starts_with_value("docs.")`,
`TsValidationRule::in_list(["draft", "published"])`,
`TsValidationRule::app_enum::<UserStatus>()`,
`TsValidationRule::uuid_version(4)`, and
`TsValidationRule::max_file_size(2048)` instead of hand-writing metadata param
keys.
For item and child object metadata, use `TsValidationRule::each([...])` and
`TsValidationRule::nested(child_schema)` rather than manually constructing
control rule codes.
For backend-only validation metadata, use helpers that mark the rule server-only
for you: `TsValidationRule::unique("users", "email")`,
`TsValidationRule::exists("countries", "id")`,
`TsValidationRule::after_hook("validate_payload")`,
`TsValidationRule::custom(MOBILE)`,
`TsValidationRule::image()`, `TsValidationRule::allowed_mimes([...])`,
`TsValidationRule::max_dimensions(width, height)`, and
`TsValidationRule::min_dimensions(width, height)`.
Known backend-owned server-only helpers are still shape-checked during export:
`unique` / `exists` need `table` and `column`, `allowed_mimes` needs at least
one value, dimensions need integer `width` / `height`, and `after_hook` needs a
non-empty hook id. Unknown custom server-only rule ids remain free-form.
Upload MIME and extension allow-list values must be non-empty and trimmed; use
`"png"` and `"image/png"` rather than padded strings.
`custom(...)` accepts a typed `ValidationRuleId` and emits the same server-only
custom-rule metadata shape as `#[validate(rule(MOBILE))]`.
`app_enum::<E>()` reads accepted inputs from
`FoundryAppEnum::accepted_keys()` and emits the same client-checkable
`app_enum` values as `#[validate(app_enum(E))]`, including aliases and
integer-backed enums.
Registered manual validation metadata projects supported built-in helpers into
the same OpenAPI JSON Schema constraints as `#[derive(Validate)]`, including
required arrays/query parameter flags, string length/patterns, array item
counts, numeric bounds, enums, per-item `each(...)` constraints, embedded
`nested` child schemas, strict `deny_unknown_fields()` object contracts, and
upload vendor extensions.
For map/object key contracts, use `TsValidationRule::required_keys(["timezone",
"locale"])`; it emits the same `required_keys` rule and `values` metadata as
`#[validate(required_keys(...))]`, and OpenAPI object schemas also receive the
matching `"required"` keys.
For strict manual object contracts, call
`TsValidationSchema::new().deny_unknown_fields().known_fields([...])`, or the
one-call `deny_unknown_fields_with_known_fields([...])` helper, so generated
route helpers reject keys that the backend does not accept. Include public
fields that have no validation rules in `known_fields`; derived `Validate`
metadata fills that list automatically.

This keeps generated TypeScript route helpers, DTO validation comments, and
OpenAPI request schemas aligned with the manual backend validator. Use
JSON/serde wire field names in manual metadata. Mark database-backed, custom, or
async checks with `.server_only()` so clients can display the contract without
trying to execute it outside the backend; export rejects unknown non-server-only
rule codes because the generated TypeScript runtime cannot honestly prevalidate
them. Schema-level manual rules added with `TsValidationSchema::rule(...)` must
also be server-only because generated browser validation currently executes
field-level rules only.
Generated `FoundryEndpoint.ts` also exports `validateFoundrySchema(...)`,
`validateFoundrySchemaField(...)`, `assertFoundrySchemaValid(...)`,
`foundryValidationResultFromError(...)`, and `FoundryValidationResult` utilities
such as `emptyFoundryValidationResult()`, `foundryValidationFirstMessage(...)`,
`foundryValidationFirstMessageOrNull(...)`,
`foundryValidationFirstFieldMessage(...)`,
`foundryValidationFirstFieldMessageOrNull(...)`,
`foundryValidationFieldCodes(...)`,
`foundryValidationFieldHasErrors(...)`, `foundryValidationFieldState(...)`,
`foundryValidationFieldStateHasErrorCode(...)`,
`foundryValidationFieldStateCountWithErrorCode(...)`,
`foundryValidationFieldStates(...)`,
`foundryValidationFieldStatesWithErrorCode(...)`,
`foundryValidationHasFieldStateWithErrorCode(...)`,
`foundryValidationHasInvalidFields(...)`,
`foundryValidationInvalidFields(...)`,
`foundryValidationFirstInvalidField(...)`,
`foundryValidationFirstInvalidFieldOrNull(...)`,
`foundryValidationInvalidFieldStates(...)`,
`foundryValidationFirstFieldStateWithErrorCode(...)`,
`foundryValidationFirstFieldStateWithErrorCodeOrNull(...)`,
`foundryValidationFirstInvalidFieldState(...)`,
`foundryValidationFirstInvalidFieldStateOrNull(...)`,
`foundryEndpointDirtyFields(...)`,
`foundryEndpointHasDirtyFields(...)`,
`foundryEndpointFirstDirtyField(...)`,
`foundryEndpointFirstDirtyFieldOrNull(...)`,
`foundryEndpointDirtyFieldStates(...)`,
`foundryEndpointFirstDirtyFieldState(...)`,
`foundryEndpointFirstDirtyFieldStateOrNull(...)`,
`foundryEndpointTouchedFields(...)`,
`foundryEndpointHasTouchedFields(...)`,
`foundryEndpointFirstTouchedField(...)`,
`foundryEndpointFirstTouchedFieldOrNull(...)`,
`foundryEndpointTouchedFieldStates(...)`,
`foundryEndpointFirstTouchedFieldState(...)`,
`foundryEndpointFirstTouchedFieldStateOrNull(...)`,
`foundryEndpointVisibleErrorFieldStates(...)`,
`foundryEndpointVisibleErrorFieldStateCount(...)`,
`foundryEndpointHasVisibleErrorFieldStates(...)`,
`foundryEndpointFirstVisibleErrorFieldState(...)`,
`foundryEndpointFieldStateHasVisibleErrorCode(...)`,
`foundryEndpointVisibleErrorFieldStatesWithErrorCode(...)`,
`foundryEndpointVisibleErrorFieldStateCountWithErrorCode(...)`,
`foundryEndpointHasVisibleErrorFieldStatesWithErrorCode(...)`,
`foundryEndpointFirstVisibleErrorFieldStateWithErrorCode(...)`,
`foundryValidationFieldHasErrorCode(...)`,
`foundryValidationFieldStateHasErrors(...)`,
`foundryValidationFieldStateMessages(...)`,
`foundryValidationFieldStateFirstMessage(...)`,
`foundryValidationFieldStateFirstMessageOrNull(...)`,
`foundryValidationFieldStateMessageCount(...)`,
`foundryValidationFieldStateMessageCountWithCode(...)`,
`foundryValidationFieldStateDetails(...)`,
`foundryValidationFieldStateFirstDetail(...)`,
`foundryValidationFieldStateFirstDetailOrNull(...)`,
`foundryValidationFieldStateDetailCount(...)`,
`foundryValidationFieldStateDetailCountWithCode(...)`,
`foundryValidationFieldStateCodes(...)`,
`foundryValidationFieldStateFirstCode(...)`,
`foundryValidationFieldStateFirstCodeOrNull(...)`,
`foundryValidationFieldStateCodeCount(...)`,
`foundryValidationFieldStateHasMessages(...)`,
`foundryValidationFieldStateHasDetails(...)`,
`foundryValidationFieldStateHasCodes(...)`,
`foundryValidationFieldStateDetailsWithCode(...)`,
`foundryValidationFieldStateHasDetailWithCode(...)`,
`foundryValidationFieldStateMessagesWithCode(...)`,
`foundryValidationFieldStateHasMessageWithCode(...)`,
`foundryValidationFieldStateFirstDetailWithCode(...)`,
`foundryValidationFieldStateFirstDetailWithCodeOrNull(...)`,
`foundryValidationFieldStateFirstMessageWithCode(...)`,
`foundryValidationFieldStateFirstMessageWithCodeOrNull(...)`,
`foundryValidationFieldMessageCount(...)`,
`foundryValidationFieldMessageCountWithCode(...)`,
`foundryValidationFieldDetailCount(...)`,
`foundryValidationFieldDetailCountWithCode(...)`,
`foundryValidationFieldCodeCount(...)`,
`foundryValidationFieldDetailsWithCode(...)`,
`foundryValidationFieldMessagesWithCode(...)`,
`foundryValidationFirstFieldMessageWithCode(...)`,
`foundryValidationFirstFieldMessageWithCodeOrNull(...)`,
`foundryValidationFirstFieldDetail(...)`,
`foundryValidationFirstFieldDetailOrNull(...)`,
`foundryValidationFirstFieldDetailWithCode(...)`,
`foundryValidationFirstFieldDetailWithCodeOrNull(...)`,
`foundryValidationFirstFieldCode(...)`,
`foundryValidationFirstFieldCodeOrNull(...)`,
`foundryValidationErrorFields(...)`,
`foundryValidationErrorMessages(...)`, `foundryValidationErrorMessagesWithCode(...)`,
`foundryValidationErrorDetails(...)`,
`foundryValidationErrorDetailsWithCode(...)`, `foundryValidationErrorCodes(...)`,
`foundryValidationHasErrorCode(...)`, `foundryValidationFieldsWithErrorCode(...)`,
`foundryValidationFirstFieldWithErrorCode(...)`,
`foundryValidationFirstFieldWithErrorCodeOrNull(...)`,
`foundryValidationFirstErrorField(...)`,
`foundryValidationFirstErrorFieldOrNull(...)`,
`foundryValidationFirstErrorMessage(...)`,
`foundryValidationFirstErrorMessageOrNull(...)`,
`foundryValidationFirstErrorMessageWithCode(...)`,
`foundryValidationFirstErrorMessageWithCodeOrNull(...)`,
`foundryValidationFirstErrorDetail(...)`,
`foundryValidationFirstErrorDetailOrNull(...)`,
`foundryValidationFirstErrorDetailWithCode(...)`,
`foundryValidationFirstErrorDetailWithCodeOrNull(...)`,
`foundryValidationFirstErrorCode(...)`, and
`foundryValidationFirstErrorCodeOrNull(...)`.
Use the `OrNull` first-value helpers when a frontend store wants an explicit
nullable contract instead of normalizing generated `undefined` results with
local `?? null` wrappers.
When normalizing an existing generated validation error bag or
`FoundryValidationClientError`, `foundryValidationResultFromError(...)` preserves
`errorDetails` rule codes instead of rebuilding every detail as `invalid`.
Generated schema validators compute `valid` from both `errors` and
`errorDetails`, so detail-only validation bags remain invalid instead of
appearing successful.
Summary and field message helpers also use `errorDetails` messages when a custom
store has detailed errors without a matching `errors` message map.
It also exports validation schema selectors such as
`foundryValidationSchemaFields(...)`,
`foundryValidationSchemaReachableFields(...)`,
`foundryValidationSchemaReachableFieldNames(...)`,
`foundryValidationSchemaReachableRules(...)`,
`foundryValidationSchemaReachableRuleCodes(...)`,
`foundryValidationSchemaFieldStateFields(...)`,
`foundryValidationSchemaField(...)`,
`foundryValidationSchemaFieldOrNull(...)`,
`foundryValidationSchemaFieldNameOrNull(...)`,
`foundryValidationSchemaFieldRules(...)`,
`foundryValidationSchemaFieldRuleCodes(...)`,
`foundryValidationSchemaFieldRule(...)`,
`foundryValidationSchemaFieldRuleOrNull(...)`,
`foundryValidationSchemaFieldRuleParam(...)`,
`foundryValidationSchemaFieldRuleValues(...)`,
`foundryValidationSchemaFieldRuleNestedRules(...)`,
`foundryValidationSchemaFieldRuleSchema(...)`,
`foundryValidationSchemaRule(...)`, `foundryValidationSchemaRuleOrNull(...)`,
`foundryValidationSchemaRuleParam(...)`,
`foundryValidationSchemaRuleValues(...)`,
`foundryValidationSchemaRuleNestedRules(...)`,
`foundryValidationSchemaRuleSchema(...)`, `foundryValidationRuleParam(...)`,
`foundryValidationRuleValues(...)`, `foundryValidationRuleNestedRules(...)`,
`foundryValidationRuleSchema(...)`, `foundryValidationSchemaMessages(...)`,
`foundryValidationSchemaMessageFields(...)`,
`foundryValidationSchemaMessageRuleCodes(...)`,
`foundryValidationSchemaMessagesByField(...)`,
`foundryValidationSchemaMessagesByRule(...)`,
`foundryValidationSchemaMessage(...)`, `foundryValidationSchemaMessageOrNull(...)`,
`foundryValidationSchemaAttributes(...)`,
`foundryValidationSchemaAttributeFields(...)`,
`foundryValidationSchemaAttributesByField(...)`,
`foundryValidationSchemaAttribute(...)`,
`foundryValidationSchemaCustomMessage(...)`,
`foundryValidationSchemaCustomMessageOrNull(...)`,
`foundryValidationSchemaAttributeOrNull(...)`,
`foundryValidationSchemaFieldLabel(...)`,
`foundryValidationSchemaFieldLabels(...)`,
`foundryValidationSchemaRuleMessage(...)`,
`FoundryValidationContainers`, `isFoundryValidationContainer(...)`,
`foundryValidationContainerOrNull(...)`, `foundryValidationSchemaIsNullable(...)`,
`foundryValidationSchemaContainer(...)`, `foundryValidationSchemaContainers(...)`,
`foundryValidationSchemaContainerCount(...)`,
`foundryValidationSchemaFirstContainer(...)`,
`foundryValidationSchemaHasContainers(...)`,
`foundryValidationSchemaHasRootContainer(...)`,
`foundryValidationSchemaHasContainer(...)`,
`FoundryValidationFieldValueKinds`, `isFoundryValidationFieldValueKind(...)`,
`foundryValidationFieldValueKindOrNull(...)`,
`foundryValidationSchemaFieldValueKindEntries(...)`,
`foundryValidationSchemaFieldValueKind(...)`,
`foundryValidationSchemaFieldValueKindOrNull(...)`,
`foundryValidationSchemaHasFieldValueKind(...)`,
`foundryValidationSchemaFirstFieldValueKind(...)`,
`foundryValidationSchemaItemsAreNullable(...)`,
`foundryValidationSchemaNullableItems(...)`,
`foundryValidationSchemaKnownFields(...)`,
`foundryValidationSchemaStrictFields(...)`,
`foundryValidationSchemaHasKnownField(...)`,
`foundryValidationSchemaKnownFieldOrNull(...)`,
`foundryValidationSchemaDeniesUnknownFields(...)`,
`foundryValidationSchemaAllowsUnknownFields(...)`, and
`foundryValidationSchemaUnknownFields(...)`, so dynamic form builders and
validation docs can read backend-owned rule, message, attribute, label-map,
container, nullability, and strict-field metadata without maintaining local
schema-scanning helpers. Field lookup and field rule selectors resolve reachable
nested child paths such as `primaryAddress.streetName` while
`foundryValidationSchemaFields(...)` remains the direct immediate-field list for
the current schema. `*OrNull(...)` schema lookup helpers normalize runtime field
names, rule codes, message keys, attribute fields, and known-field checks into
explicit `null` results without local optional-chaining wrappers. Use
`foundryValidationSchemaReachableFields(...)` or
`foundryValidationSchemaReachableFieldNames(...)` when a form builder wants the
complete parent-prefixed nested field contract. Message, attribute, label, and
rule-message selectors also include parent-prefixed nested child metadata such as
`primaryAddress.streetName` or `previousAddresses.streetName`; concrete indexed
reads such as `previousAddresses[0].streetName` resolve against that same
backend-owned nested metadata. Filtered message selectors and copy selectors
accept plain field strings, so server-returned paths or concrete collection
paths can be passed directly without app-local casts.
`foundryValidationSchemaUnknownFields(...)` traverses
root array, map, and collection request containers and returns the same
container-prefixed paths as generated validation errors, such as `[0].extra`,
`[tenant].extra`, or `items[0].extra`, typed as
`FoundryRequestField<TRequest>` so they can flow back into generated endpoint
field APIs without casts. Selectors that return schema fields, rules, nested rule
schema, messages, attributes, container lists, nullable-item lists, or strict-field
lists clone those values, so mutating a selector result cannot
mutate the generated validation contract.
Count helpers mirror the same schema metadata selectors:
`foundryValidationSchemaRuleCount(...)`,
`foundryValidationSchemaClientRuleCount(...)`,
`foundryValidationSchemaControlRuleCount(...)`,
`foundryValidationSchemaServerOnlyRuleCount(...)`,
`foundryValidationSchemaCustomRuleCount(...)`,
`foundryValidationSchemaFieldCount(...)`,
`foundryValidationSchemaFieldRuleCount(...)`,
`foundryValidationSchemaFieldClientRuleCount(...)`,
`foundryValidationSchemaFieldControlRuleCount(...)`,
`foundryValidationSchemaFieldServerOnlyRuleCount(...)`,
`foundryValidationSchemaFieldCustomRuleCount(...)`,
`foundryValidationSchemaFieldReferenceCount(...)`,
field grouping counts such as
`foundryValidationSchemaFieldCountWithClientRules(...)`,
`foundryValidationSchemaFieldCountWithServerOnlyRules(...)`,
`foundryValidationSchemaFieldCountWithRuleCode(...)`,
`foundryValidationSchemaFieldCountReferencing(...)`,
field-name grouping counts such as
`foundryValidationSchemaFieldNameCountWithClientRules(...)`,
`foundryValidationSchemaFieldNameCountWithServerOnlyRules(...)`,
`foundryValidationSchemaFieldNameCountWithRuleCode(...)`,
`foundryValidationSchemaFieldNameCountReferencing(...)`,
required/nullability counts, message/attribute counts, root container counts,
nullable-item counts, and known/strict-field counts. Rule-row helpers also expose
`foundryValidationRuleValueCount(...)`,
`foundryValidationRuleFieldReferenceCount(...)`, and
`foundryValidationRuleNestedRuleCount(...)`, so diagnostics and form-builder
badges do not need local `.length` wrappers over backend-owned metadata arrays.
Schema and field rule wrappers mirror those count helpers with
`foundryValidationSchemaRuleValueCount(...)`,
`foundryValidationSchemaFieldRuleValueCount(...)`,
`foundryValidationSchemaFieldRuleFieldReferenceCount(...)`, and
`foundryValidationSchemaFieldRuleNestedRuleCount(...)`.
Presence helpers mirror those counts with generated booleans such as
`foundryValidationSchemaHasRules(...)`,
`foundryValidationSchemaHasClientRules(...)`,
`foundryValidationSchemaHasServerOnlyRules(...)`,
`foundryValidationSchemaHasCustomRules(...)`,
`foundryValidationSchemaHasFields(...)`,
`foundryValidationSchemaFieldHasRules(...)`,
`foundryValidationSchemaHasFieldsWithRuleCode(...)`,
`foundryValidationSchemaHasFieldNamesWithRuleCode(...)`,
`foundryValidationSchemaHasFieldsReferencing(...)`,
`foundryValidationSchemaHasFieldNamesReferencing(...)`,
`foundryValidationSchemaHasMessages(...)`,
`foundryValidationSchemaHasAttributes(...)`,
`foundryValidationSchemaHasKnownFields(...)`,
`foundryValidationSchemaHasStrictFields(...)`, and
`foundryValidationRuleHasValues(...)`, plus rule-wrapper checks such as
`foundryValidationSchemaHasRuleValues(...)`,
`foundryValidationSchemaFieldHasRuleFieldReferences(...)`, and
`foundryValidationSchemaFieldHasRuleNestedRules(...)`, so form sections can
branch on backend-owned schema metadata without local `count(...) > 0` wrappers.
First-value helpers complete the same metadata set with generated reads such as
`foundryValidationSchemaFirstRule(...)`,
`foundryValidationSchemaFirstField(...)`,
`foundryValidationSchemaFieldFirstRule(...)`,
`foundryValidationSchemaFirstFieldWithRuleCode(...)`,
`foundryValidationSchemaFirstMessage(...)`,
`foundryValidationSchemaFirstAttribute(...)`,
`foundryValidationSchemaFirstKnownField(...)`, and
`foundryValidationRuleFirstValue(...)`, so inspectors and form builders do not
need local `[0]` access over generated metadata arrays.
Endpoint instances mirror the same backend-owned schema metadata with
`validationSchemaFields()`, `validationSchemaFieldRules(field)`,
`validationSchemaFieldFirstRule(field)`,
`validationSchemaFieldsWithRuleCode(code)`,
`validationSchemaFirstFieldWithRuleCode(code)`,
`validationSchemaMessages(field?)`, `validationSchemaFirstMessage(field?)`,
`validationSchemaAttributes()`, `validationSchemaFieldLabel(field)`,
`validationSchemaCustomMessage(field, rule)`,
`validationSchemaRuleMessage(field, rule)`, `validationSchemaKnownFields()`,
and `validationSchemaUnknownFields(data?)`.
Route-local form builders can stay on the generated endpoint object instead of
wrapping `endpoint.validation` with app-local selector, count, presence, or
first-item helpers.
The same runtime exports `FoundryClientValidationRuleCodes`,
`FoundryValidationControlRuleCodes`, `FoundryValidationRequiredRuleCodes`,
`FoundryValidationFieldReferenceRuleCodes`, `FoundryValidationRuntimeRuleCodes`,
`foundryClientValidationRuleCodes()`, `foundryValidationControlRuleCodes()`,
`foundryValidationRequiredRuleCodes()`,
`foundryValidationFieldReferenceRuleCodes()`,
`foundryValidationRuntimeRuleCodes()`,
matching count/presence/first helpers such as
`foundryClientValidationRuleCodeCount()`,
`foundryValidationControlHasRuleCodes()`,
`foundryValidationRequiredFirstRuleCode()`,
`foundryValidationFieldReferenceRuleCodeCount()`, and
`foundryValidationRuntimeFirstRuleCode()`,
`isFoundryClientValidationRuleCode(...)`,
`isFoundryValidationControlRuleCode(...)`,
`isFoundryValidationRequiredRuleCode(...)`,
`isFoundryValidationFieldReferenceRuleCode(...)`,
`isFoundryValidationRuntimeRuleCode(...)`, and
safe parser helpers such as `foundryClientValidationRuleCodeOrNull(...)`,
`foundryValidationControlRuleCodeOrNull(...)`,
`foundryValidationRequiredRuleCodeOrNull(...)`,
`foundryValidationFieldReferenceRuleCodeOrNull(...)`, and
`foundryValidationRuntimeRuleCodeOrNull(...)`, plus
`foundryValidationRuleIsClientCheckable(...)`,
`foundryValidationRuleIsControl(...)`, `foundryValidationRuleIsServerOnly(...)`,
and schema/field filters such as `foundryValidationSchemaFieldClientRules(...)`,
`foundryValidationSchemaFieldControlRules(...)`, and
`foundryValidationSchemaFieldServerOnlyRules(...)`, so frontend tooling can
distinguish browser-checkable metadata, validation controls, and server-only
custom, database, and content-sniffing rules without copying Foundry's rule-code
list.
Generated validation rule-code lists are frozen at runtime, so direct mutation
cannot change backend-owned validation capability metadata. Rule-code selector
helpers return fresh arrays, so form builders and docs tools can add local
labels or display grouping to selector results.
Use `foundryValidationSchemaFieldsWithClientRules(...)`,
`foundryValidationSchemaFieldNamesWithClientRules(...)`,
`foundryValidationSchemaFieldsWithControlRules(...)`,
`foundryValidationSchemaFieldNamesWithControlRules(...)`,
`foundryValidationSchemaFieldsWithServerOnlyRules(...)`, and
`foundryValidationSchemaFieldNamesWithServerOnlyRules(...)` when a form or docs
UI needs fields grouped by browser-checkable, control-only, or server-only
validation metadata. Use `foundryValidationSchemaFieldHasClientRules(...)`,
`foundryValidationSchemaFieldHasControlRules(...)`,
`foundryValidationSchemaFieldHasServerOnlyRules(...)`,
`foundryValidationSchemaFieldHasCustomRules(...)`, and
`foundryValidationSchemaFieldHasFieldReferences(...)` when a form component needs
the same classification for one field without reading helper-array lengths.
Use `foundryValidationSchemaReachableRules(...)`,
`foundryValidationSchemaReachableRuleCodes(...)`,
`foundryValidationSchemaReachableRulesWithCode(...)`, and reachable
classification helpers such as `foundryValidationSchemaReachableClientRules(...)`
when tooling needs the complete nested rule contract across schema rules,
fields, `each(...)`, and `nested` child schemas. `foundryValidationSchemaRules(...)`
remains the immediate root-schema rule list; use
`foundryValidationSchemaRuleCodes(...)`,
`foundryValidationSchemaRulesWithCode(...)`,
`foundryValidationSchemaRuleCodeCount(...)`, and
`foundryValidationSchemaHasRuleCode(...)` when tooling needs immediate
root-schema rule-code summaries or filters. Rule-code summary selectors return
first-seen unique backend rule codes; use the corresponding rule row selectors
when repeated occurrences matter.
Use `foundryValidationSchemaFieldHasRuleCode(...)`,
`foundryValidationSchemaFieldsWithRuleCode(...)`, or
`foundryValidationSchemaFieldNamesWithRuleCode(...)` when a form needs fields
with a backend-owned rule such as `required`, `nullable`, `email`, or a custom
rule id. Use `foundryValidationRuleIsNullable(...)`,
`foundryValidationRuleIsBail(...)`, `foundryValidationRuleIsRequired(...)`,
`foundryValidationRuleIsConditionalRequired(...)`,
`foundryValidationRuleIsRequiredRule(...)`,
`foundryValidationSchemaFieldIsNullable(...)`,
`foundryValidationSchemaFieldIsRequired(...)`,
`foundryValidationSchemaFieldHasConditionalRequiredRule(...)`,
`foundryValidationSchemaFieldHasRequiredRule(...)`,
`foundryValidationSchemaNullableFields(...)`,
`foundryValidationSchemaNullableFieldNames(...)`,
`foundryValidationSchemaRequiredFields(...)`,
`foundryValidationSchemaRequiredFieldNames(...)`,
`foundryValidationSchemaConditionallyRequiredFields(...)`,
`foundryValidationSchemaConditionallyRequiredFieldNames(...)`,
`foundryValidationSchemaFieldsWithRequiredRules(...)`, and
`foundryValidationSchemaFieldNamesWithRequiredRules(...)` when a form needs
required badges, conditional-required hints, nullable inputs, or generated
validation-control state without scanning raw rule codes. `filled` is included
in required/presence helpers because generated and backend validation both reject
absent or empty `filled` values; `required_keys` is an object-key rule and is not
included in the field-presence helper list. Use
`foundryValidationFieldReferenceMatches(...)`,
`foundryValidationRuleFieldReferences(...)`,
`foundryValidationRuleFieldReferencesForField(...)`,
`foundryValidationRuleHasFieldReferences(...)`,
`foundryValidationRuleReferencesField(...)`,
`foundryValidationRuleReferencesFieldForField(...)`,
`foundryValidationSchemaFieldRuleFieldReferences(...)`,
`foundryValidationSchemaFieldRuleReferencesField(...)`,
`foundryValidationSchemaFieldReferences(...)`,
`foundryValidationSchemaFieldDependsOn(...)`,
`foundryValidationSchemaFieldsReferencing(...)`, and
`foundryValidationSchemaFieldNamesReferencing(...)` when a form needs to inspect
fields affected by backend-owned sibling-field rules such as `required_if`,
`required_with_all`, `confirmed`, temporal comparisons, prohibitions, or
accepted/declined conditionals without copying Foundry's dependency rule list or
parsing `params.other`. Endpoint instance methods preserve
`FoundryRequestField<TRequest>` return types for field-reference lists and first
items, so adapter code keeps the generated request field union when composing
dependent-input UIs. Use `foundryValidationRuleFieldReferencesForField(...)`
when a rule is attached to a concrete nested/indexed field and the UI needs the
sibling references resolved to the same field path; count, first, nullable-first,
presence, and predicate companions are available. Use
`foundryValidationSchemaFieldRuleReferencesField(...)` when a form needs to test
whether one specific rule code on a field references another field. Use `foundryValidationDependentFieldName(...)` and
`foundryValidationSchemaDependentFieldNames(...)` when the UI needs the concrete
runtime field paths to revalidate after a change; matching count, first, and
presence helpers are available as
`foundryValidationSchemaDependentFieldNameCount(...)`,
`foundryValidationSchemaFirstDependentFieldName(...)`, and
`foundryValidationSchemaHasDependentFieldNames(...)`. Generated endpoint
instances use the same mapped metadata through `validateFieldAndDependents(...)`
when a changed field should refresh itself and fields that reference it.
Dependency matching uses the same path-family rules as validation reads, so a
reference to `children.name` matches `children[0].name`, parent object changes
match child references, and root-container paths such as `[0].email` or
`items[0].email` match an `email` reference while dependent refreshes preserve
the same address, for example `[0].emailConfirmation` or
`items[0].emailConfirmation`. References inside `nested` and `each(nested)`
child schemas are parent-prefixed before dependency matching, so a child rule
such as `confirmed(accessCodeConfirmation)` under `primaryAddress.accessCode`
is exposed as `primaryAddress.accessCodeConfirmation`, while
`previousAddresses[0].accessCodeConfirmation` refreshes
`previousAddresses[0].accessCode`. Browser-side conditional and sibling rule
execution resolves the same references through the generated field-path reader
after checking for an exact flat key, so rules can target `profile.enabled`,
`children[0].status`, or `children.status` without copying those values into
top-level shadow fields. For custom rules, use `foundryValidationRuleCustomRuleId(...)`,
`foundryValidationRuleHasCustomRuleId(...)`, `foundryValidationRuleIsCustom(...)`,
`foundryValidationSchemaCustomRuleIds(...)`,
`foundryValidationSchemaCustomRulesWithRuleId(...)`,
`foundryValidationSchemaReachableCustomRulesWithRuleId(...)`,
`foundryValidationSchemaFieldCustomRulesWithRuleId(...)`,
`foundryValidationSchemaFieldCustomRules(...)`,
`foundryValidationSchemaFieldsWithCustomRules(...)`, or
`foundryValidationSchemaFieldNamesWithCustomRules(...)` instead of reading
`params.rule` directly or scanning every field rule. Custom-rule ID selectors
return first-seen unique backend rule IDs; use the corresponding custom-rule row
selectors when repeated occurrences matter. Field classification selectors for
client, control, server-only, custom, required, nullable, and rule-code groups
return reachable nested-schema field paths and include reachable `each(...)`
item rules, so `primaryAddress.streetName` appears as the
required/client-checkable field for a nested child rule while a collection field
with `each(required)` is still reported by the client-rule helpers without local
recursive rule scans. Lookup and rule-code selectors such as
`foundryValidationSchemaFieldRule(...)`,
`foundryValidationSchemaFieldHasRuleCode(...)` and
`foundryValidationSchemaFieldsWithRuleCode(...)` use the same reachable-rule
view, so `each(required)` is found by `required` rule-code queries.
Generated endpoint instances expose the same reads through
`validationResult()`, `routeUrl()`, `routeUrlOrNull()`, `submitUrl()`,
`submitUrlOrNull()`, `submitMode()`, `submitModeOrNull()`,
`hasResponse()`, `responseStatuses()`, `responseStatus()`, `hasResponseStatus(...)`,
`responseMetadataForStatus(...)`, `hasDocumentedResponseStatus(...)`,
`hasPendingSubmissions(...)`,
`hasServerError()`, `hasErrorResponse()`, `hasValidationErrorResponse()`,
`hasErrors()`,
`errorFieldCount()`, `hasErrorFields()`, `errorFieldCountWithCode(...)`,
`hasErrorFieldWithCode(...)`, `errorFields()`,
`errorMessageCount()`, `hasErrorMessages()`,
`errorMessageCountWithCode(...)`, `hasErrorMessageWithCode(...)`, `errorMessages()`,
`errorMessagesWithCode(...)`, `errorDetailCount()`,
`hasErrorDetails()`, `errorDetailCountWithCode(...)`,
`hasErrorDetailWithCode(...)`, `allErrorDetails()`,
`errorDetailsWithCode(...)`, `errorCodeCount()`, `hasErrorCodes()`, `errorCodes()`,
`hasErrorCode(...)`, `fieldsWithErrorCode(...)`, `firstFieldWithErrorCode(...)`,
`firstFieldWithErrorCodeOrNull(...)`, `firstErrorField()`,
`firstErrorFieldOrNull()`, `firstErrorMessage()`,
`firstErrorMessageOrNull()`, `firstErrorMessageWithCode(...)`,
`firstErrorMessageWithCodeOrNull(...)`, `firstErrorDetail()`,
`firstErrorDetailOrNull()`, `firstErrorDetailWithCode(...)`,
`firstErrorDetailWithCodeOrNull(...)`, `firstErrorCode()`,
`firstErrorCodeOrNull()`,
`fieldHasErrors(...)`, `fieldHasMessages(...)`, `fieldHasDetails(...)`,
`fieldHasCodes(...)`, `fieldHasDetailWithCode(...)`,
`fieldHasMessageWithCode(...)`, `fieldHasVisibleErrors(...)`, `fieldState(...)`, `fieldStates(...)`,
`visibleFieldMessages(...)`, `firstVisibleFieldMessage(...)`,
`firstVisibleFieldMessageOrNull(...)`, `visibleFieldDetails(...)`,
`firstVisibleFieldDetail(...)`, `firstVisibleFieldDetailOrNull(...)`,
`visibleFieldCodes(...)`, `firstVisibleFieldCode(...)`,
`firstVisibleFieldCodeOrNull(...)`,
`visibleFieldDetailsWithCode(...)`, `visibleFieldMessagesWithCode(...)`,
`firstVisibleFieldDetailWithCode(...)`, `firstVisibleFieldDetailWithCodeOrNull(...)`,
`firstVisibleFieldMessageWithCode(...)`, `firstVisibleFieldMessageWithCodeOrNull(...)`,
`dirtyFieldCount(...)`, `hasDirtyFields(...)`, `dirtyFields(...)`, `firstDirtyField(...)`,
`firstDirtyFieldOrNull(...)`, `dirtyFieldStates(...)`,
`firstDirtyFieldState(...)`, `firstDirtyFieldStateOrNull(...)`,
`touchedFieldCount(...)`, `hasTouchedFields(...)`, `touchedFields(...)`, `firstTouchedField(...)`,
`firstTouchedFieldOrNull(...)`, `touchedFieldStates(...)`,
`firstTouchedFieldState(...)`, `firstTouchedFieldStateOrNull(...)`,
`invalidFieldCount(...)`, `hasInvalidFields(...)`, `invalidFields(...)`, `firstInvalidField(...)`,
`firstInvalidFieldOrNull(...)`, `invalidFieldStates(...)`,
`firstInvalidFieldState(...)`, `firstInvalidFieldStateOrNull(...)`,
`fieldStateCountWithErrorCode(...)`, `hasFieldStateWithErrorCode(...)`,
`fieldStatesWithErrorCode(...)`, `firstFieldStateWithErrorCode(...)`,
`firstFieldStateWithErrorCodeOrNull(...)`,
`visibleErrorFieldStates(...)`, `visibleErrorFieldStateCount(...)`,
`hasVisibleErrorFieldStates(...)`, `firstVisibleErrorFieldState(...)`,
`firstVisibleErrorFieldStateOrNull(...)`,
`visibleErrorFieldStatesWithErrorCode(...)`,
`visibleErrorFieldStateCountWithErrorCode(...)`,
`hasVisibleErrorFieldStatesWithErrorCode(...)`,
`firstVisibleErrorFieldStateWithErrorCode(...)`,
`firstVisibleErrorFieldStateWithErrorCodeOrNull(...)`,
`hasVisibleErrors(...)`, `hasVisibleErrorCode(...)`,
`hasVisibleErrorFields(...)`, `visibleErrorFieldCount(...)`,
`hasVisibleErrorFieldWithCode(...)`, `visibleErrorFieldCountWithCode(...)`,
`visibleErrorFields(...)`, `firstVisibleErrorField(...)`,
`firstVisibleErrorFieldOrNull(...)`, `visibleErrorFieldsWithCode(...)`,
`firstVisibleErrorFieldWithCode(...)`, `firstVisibleErrorFieldWithCodeOrNull(...)`,
`hasVisibleErrorMessages(...)`, `visibleErrorMessageCount(...)`,
`hasVisibleErrorMessageWithCode(...)`, `visibleErrorMessageCountWithCode(...)`,
`visibleErrorMessages(...)`, `firstVisibleErrorMessage(...)`,
`firstVisibleErrorMessageOrNull(...)`,
`hasVisibleErrorDetails(...)`, `visibleErrorDetailCount(...)`,
`hasVisibleErrorDetailWithCode(...)`, `visibleErrorDetailCountWithCode(...)`,
`visibleErrorDetails(...)`, `firstVisibleErrorDetail(...)`,
`firstVisibleErrorDetailOrNull(...)`,
`hasVisibleErrorCodes(...)`, `visibleErrorCodeCount(...)`,
`visibleErrorCodes(...)`, `firstVisibleErrorCode(...)`,
`firstVisibleErrorCodeOrNull(...)`,
`visibleErrorDetailsWithCode(...)`, `visibleErrorMessagesWithCode(...)`,
`firstVisibleErrorDetailWithCode(...)`, `firstVisibleErrorDetailWithCodeOrNull(...)`,
`firstVisibleErrorMessageWithCode(...)`, `firstVisibleErrorMessageWithCodeOrNull(...)`,
`firstError(...)`, `firstErrorOrNull(...)`, `firstFieldMessage(...)`,
`firstFieldMessageOrNull(...)`, `fieldCodes(...)`,
`fieldHasErrorCode(...)`, `fieldHasVisibleErrorCode(...)`,
`fieldDetailsWithCode(...)`, `fieldMessagesWithCode(...)`,
`firstFieldMessageWithCode(...)`, `firstFieldMessageWithCodeOrNull(...)`,
`firstFieldDetail(...)`, `firstFieldDetailOrNull(...)`,
`firstFieldDetailWithCode(...)`, `firstFieldDetailWithCodeOrNull(...)`,
`firstFieldCode(...)`, `firstFieldCodeOrNull(...)`, `setField(...)`,
`setFieldAndValidate(...)`, `setFieldAndValidateDependents(...)`,
`validateFields(...)`, `validateField(...)`, `validateFieldAndDependents(...)`,
`clearFieldErrors(...)`, `clearFieldError(...)`,
`clearErrors(...)`, `clearError(...)`,
`setFieldErrors(...)`, `setFieldError(...)`, `setErrors(...)`, `setError(...)`,
`touchField(...)`, `touchFields(...)`, `touchFieldAndValidate(...)`,
`touchFieldsAndValidate(...)`,
`touchFieldAndValidateDependents(...)`, `isFieldTouched(...)`,
`resetTouched(...)`, and
`applyValidationResult(...)`, plus `setInitialData(...)`,
`setInitialFields(...)`, `setInitialField(...)`, `defaults(...)`, `isDirty(...)`,
`isFieldDirty(...)`, `resetData(...)`, `resetFields(...)`, `resetField(...)`, and
`reset(...)`, `resetAndClearErrors(...)`, `resetSubmitState(...)`, `clearRecentlySuccessful(...)`, `clearProgress(...)`, plus `resetResponse(...)`, `clearResponse(...)`, `applyResponse(...)`,
`applyServerError(...)`, `submit(...)`, `prepareSubmit(...)`, `prepareSubmitOrNull(...)`, and `cancel(...)`
for generated response/error/progress lifecycle state, so route-helper forms and custom
hooks can share the same generated error-bag shape. `clearFieldErrors(...)` clears a generated field
subset in one state emission, defaulting to the generated field-state key list
when no fields are passed. That default list includes direct schema fields,
reachable nested child paths, current validation error fields, and currently
touched fields so nested DTO rows and exact indexed touched rows stay visible in
bulk state operations. `clearErrors(...)` and `clearError(...)` are
Laravel-style aliases; call `clearErrors("email", "password")` for a variadic
subset or `clearErrors()` for the whole generated error bag, including
backend-returned fields outside the generated field-state key list.
`setFieldErrors(...)` / `setFieldError(...)` replace
a generated field subset with local/client validation errors in the same
`errors` and `errorDetails` bags used by backend validation, accepting string
messages or `FieldError`-shaped `{ code, message }` objects. Route helper files
export `{RouteName}FieldErrorsInput` aliases for these local error bags.
They also export route-specific validation metadata row aliases such as
`{RouteName}ValidationRule`, `{RouteName}ValidationField`,
`{RouteName}ValidationMessage`, `{RouteName}ValidationAttribute`, and
`{RouteName}ValidationContainer`, so form builders can type backend-owned
schema rows without spelling generic `FoundryValidation*` helpers by hand.
`setErrors(...)` replaces the whole generated error bag, so `setErrors({})`
clears every field; use `setFieldErrors(...)` for partial merges.
`setError(...)` is the Laravel-style single-field alias. Applying local errors
also clears stale success/response state so an invalid generated endpoint does
not keep showing an old successful submit or response.
`resetFields(...)` restores a generated field subset
from `initialData` in one state emission while clearing touched/error state for
the same paths. `setInitialFields(...)` marks a generated field subset clean
from the current request body after partial saves without replacing the rest of
`initialData`, and `defaults(...)` is the Laravel/Inertia-style alias for the
same clean-baseline update. Generated endpoints clone full-form and field-level
clean-baseline request values when they are captured or restored, so mutating
current form data does not accidentally mutate `initialData`. `reset(...)` is
the Laravel/Inertia-style alias for restoring all data or a variadic field subset,
and `resetAndClearErrors(...)`
delegates to the same generated reset path because matching errors are already
cleared by `resetData(...)` / `resetFields(...)`. Route helpers also export aliases such as
`UserPortalLoginState`, `UserPortalLoginFieldState`, `UserPortalLoginFieldStates`,
`UserPortalLoginStateSubscriber`, and
`UserPortalLoginSubmitOptions` / `UserPortalLoginSubmitUrlOptions` so adapters
can bind to one backend-owned route contract without spelling the generic
endpoint types by hand. Endpoint snapshots
include `touched`, `processing`, `submitted`, `submitCount`,
`pendingSubmitCount`, `hasPendingSubmissions`, `wasSuccessful`, and
`recentlySuccessful` alongside validation state and the generated `fieldStates`
map. Request/form/response snapshot values such as `data`, `initialData`,
`routeParams`, `touched`, `errors`, `errorDetails`, `progress`, `response`,
`rawResponse`, `responseMetadata`, `responseStatuses`,
`responseStatusMetadata`, and `validation` are cloned so stores cannot mutate
endpoint internals by mutating the snapshot object. `dirty` is available as an
endpoint getter backed by `isDirty()`,
`processing` is a Laravel/Inertia-style alias for `busy`, and `submitted` is
available on both the endpoint instance and `state()` snapshots. `valid` and
`invalid` are also available on the endpoint instance for adapters that branch
without first creating a snapshot.
`recentlySuccessful` is set for the latest successful submit and auto-clears
after the generated submit option's `recentlySuccessfulDurationMs` value,
defaulting to 2000 milliseconds. Use `clearRecentlySuccessful()` to dismiss
that success pulse without resetting submit counters, progress, responses, or
errors.
`fieldState(...)` returns a per-field object with `valid`, `invalid`, messages,
details, codes, first message/detail/code, and endpoint display flags
(`touched`, `dirty`, `submitted`, `shouldShowErrors`) so frontend adapters do not
need local reducers around backend-owned validation state. `fieldStates()` maps
every generated schema field and current validation error field to that object
by default, so server-only, unknown-field, and exact indexed error keys stay
visible to subscribers; pass a field list when an adapter needs a smaller state
map. `state().fieldStates` exposes the generated default map directly to
snapshot subscribers, and `state().dirtyFieldCount`,
`state().hasDirtyFields`, `state().dirtyFields`, `state().firstDirtyField`,
`state().dirtyFieldStates`, `state().firstDirtyFieldState`,
`state().touchedFieldCount`, `state().hasTouchedFields`,
`state().touchedFields`, `state().firstTouchedField`,
`state().touchedFieldStates`, `state().firstTouchedFieldState`,
`state().invalidFieldCount`, `state().hasInvalidFields`, `state().invalidFields`,
`state().firstInvalidField`, `state().invalidFieldStates`,
`state().firstInvalidFieldState`,
`state().visibleErrorFieldStates`, and `state().firstVisibleErrorFieldState`
expose the common derived counts and lists from the same map. Snapshots also
expose `state().errorFieldCount`,
`state().errorFields`, `state().firstErrorField`,
`state().errorMessageCount`, `state().errorMessages`,
`state().firstErrorMessage`, `state().errorDetailCount`,
`state().allErrorDetails`, `state().firstErrorDetail`,
`state().errorCodeCount`, `state().errorCodes`, and
`state().firstErrorCode` for whole-form validation summaries, plus
`state().hasVisibleErrors`,
`state().visibleErrorFieldCount`, `state().visibleErrorFields`,
`state().firstVisibleErrorField`, `state().visibleErrorMessageCount`,
`state().visibleErrorMessages`, `state().firstVisibleErrorMessage`,
`state().visibleErrorDetailCount`, `state().visibleErrorDetails`,
`state().firstVisibleErrorDetail`, `state().visibleErrorCodeCount`,
`state().visibleErrorCodes`, and `state().firstVisibleErrorCode` for
touched/submitted summary displays, plus
`state().valid`, `state().invalid`, and `state().hasErrors` beside
`state().validation.valid`, so adapters can branch on whole-form validity
without local aliases. The same snapshot includes static
route metadata such as `state().routeName`, `state().path`,
`state().routeUrl`, `state().method`, `state().requestTransport`,
`state().requestMediaType`, `state().responseMetadata`, and
`state().responseStatuses`, so subscribed stores do not need a second
route-metadata object. `state().routeUrl` is `null` when the current stored
params cannot build a route URL. The snapshot also exposes `state().submitUrl`
for the current backend-owned generated submit URL, or `null` when it cannot be
built from the current endpoint metadata/data, and `state().submitMode` for the
resolved submit method, query/body transport, request body media type, and
query/multipart/body booleans,
`state().responseStatus`, `state().responseStatusMetadata`, and
`state().hasDocumentedResponseStatus` for the current documented response
without local status/metadata scans. Use `dirtyFieldCount(...)` /
`hasDirtyFields(...)` /
`dirtyFields(...)` / `firstDirtyField(...)` /
`firstDirtyFieldOrNull(...)` / `dirtyFieldStates(...)` /
`firstDirtyFieldState(...)` / `firstDirtyFieldStateOrNull(...)` for changed
state totals, fields, or rows,
`touchedFieldCount(...)` / `hasTouchedFields(...)` /
`touchedFields(...)` / `firstTouchedField(...)` /
`firstTouchedFieldOrNull(...)` / `touchedFieldStates(...)` /
`firstTouchedFieldState(...)` / `firstTouchedFieldStateOrNull(...)` for
interacted state totals, fields, or rows,
`invalidFieldCount(...)` / `hasInvalidFields(...)` /
`invalidFields(...)` / `firstInvalidField(...)` /
`firstInvalidFieldOrNull(...)` / `invalidFieldStates(...)` /
`firstInvalidFieldState(...)` / `firstInvalidFieldStateOrNull(...)` for invalid
state totals, fields, or rows,
`fieldStateCountWithErrorCode(...)` / `hasFieldStateWithErrorCode(...)` /
`fieldStatesWithErrorCode(...)` /
`firstFieldStateWithErrorCode(...)` /
`firstFieldStateWithErrorCodeOrNull(...)` for full state rows matching a
backend-owned rule code, and for the corresponding rule-code count/presence,
and
`visibleErrorFieldStates(...)` /
`firstVisibleErrorFieldState(...)` /
`firstVisibleErrorFieldStateOrNull(...)` when errors should wait for touched or
submitted state. Use `visibleErrorFieldStatesWithErrorCode(...)` /
`visibleErrorFieldStateCountWithErrorCode(...)` /
`hasVisibleErrorFieldStatesWithErrorCode(...)` /
`firstVisibleErrorFieldStateWithErrorCode(...)` /
`firstVisibleErrorFieldStateWithErrorCodeOrNull(...)` when the displayed-error
summary also needs rows, counts, presence, or first state for one rule code. Use
the `OrNull` variants when a store contract uses `null` for missing first values, or
`foundryEndpointFieldStateHasVisibleErrorCode(...)` for a single endpoint
field-state row. Use endpoint `fieldHasVisibleErrorCode(...)` when a field
component needs one field/rule-code boolean.
Use `foundryValidationFieldStateHasErrors(...)`,
`foundryValidationFieldStateMessages(...)`,
`foundryValidationFieldStateDetails(...)`,
`foundryValidationFieldStateCodes(...)`, first-value row helpers,
row presence helpers,
`foundryValidationFieldStateMessageCount(...)`,
`foundryValidationFieldStateDetailCount(...)`,
`foundryValidationFieldStateCodeCount(...)`,
`foundryValidationFieldStateDetailsWithCode(...)`,
`foundryValidationFieldStateMessagesWithCode(...)`, first/count `WithCode`
variants, and first-value `OrNull` variants when a component receives a
generated field-state row directly and should not duplicate optional checks,
`?? null`, or count/filter/map logic over backend-owned errors.
Use `visibleFieldMessages(...)`, `firstVisibleFieldMessage(...)`,
`firstVisibleFieldMessageOrNull(...)`,
`visibleFieldMessageCount(...)`, `visibleFieldDetails(...)`,
`firstVisibleFieldDetail(...)`, `firstVisibleFieldDetailOrNull(...)`,
`visibleFieldDetailCount(...)`, `visibleFieldCodes(...)`,
`firstVisibleFieldCode(...)`, `firstVisibleFieldCodeOrNull(...)`,
`visibleFieldCodeCount(...)`, `fieldHasVisibleMessages(...)`,
`fieldHasVisibleDetails(...)`, `fieldHasVisibleCodes(...)`, and their
`WithCode` / `OrNull` variants when a field
component should read only touched/submitted display errors without duplicating
the generated `shouldShowErrors` rule or local `.length` wrappers.
Use `hasVisibleErrors(...)`, `hasVisibleErrorCode(...)`,
`visibleErrorFieldCount(...)`, `visibleErrorFieldCountWithCode(...)`,
`visibleErrorFields(...)`, `visibleErrorMessageCount(...)`,
`visibleErrorMessageCountWithCode(...)`, `visibleErrorMessages(...)`,
`visibleErrorDetailCount(...)`, `visibleErrorDetailCountWithCode(...)`,
`visibleErrorDetails(...)`, `visibleErrorCodeCount(...)`,
`visibleErrorCodes(...)`, and their first-value / `WithCode` / `OrNull` variants when a
form-level summary needs only touched/submitted display errors without
flattening visible field-state rows locally; subscribed stores can read the same
boolean, counts, and aggregate field/message/detail/code arrays from `state()`.
Upload progress events emitted by HTTP adapters through
`config.onUploadProgress(event)` are normalized into `progress` snapshot state
and typed `onProgress(progress)` submit callbacks. Endpoint instances and
`state()` snapshots expose `hasProgress` so upload UIs can branch without local
`progress !== null` aliases, and custom upload flows can call
`clearProgress()` when they need to dismiss progress state manually.
`setField(...)`
accepts generated nested and indexed field paths too, so adapters can write
fields such as `profile.name` or `children[0].name` through the backend-owned
path contract before validating, touching, or clearing that same path.
`FoundryValidationClientError` exposes the
same `validationResult()`, `hasErrors()`, `errorFieldCount()`,
`hasErrorFields()`, `errorFieldCountWithCode(...)`,
`hasErrorFieldWithCode(...)`, `errorFields()`, `errorMessageCount()`,
`hasErrorMessages()`, `errorMessageCountWithCode(...)`,
`hasErrorMessageWithCode(...)`, `errorMessages()`,
`errorMessagesWithCode(...)`, `errorDetailCount()`,
`hasErrorDetails()`, `errorDetailCountWithCode(...)`,
`hasErrorDetailWithCode(...)`, `allErrorDetails()`,
`errorDetailsWithCode(...)`, `errorCodeCount()`, `hasErrorCodes()`, `errorCodes()`,
`hasErrorCode(...)`, `fieldsWithErrorCode(...)`, `firstFieldWithErrorCode(...)`,
`firstFieldWithErrorCodeOrNull(...)`, `firstErrorField()`,
`firstErrorFieldOrNull()`, `firstErrorMessage()`,
`firstErrorMessageOrNull()`, `firstErrorMessageWithCode(...)`,
`firstErrorMessageWithCodeOrNull(...)`, `firstErrorDetail()`,
`firstErrorDetailOrNull()`, `firstErrorDetailWithCode(...)`,
`firstErrorDetailWithCodeOrNull(...)`, `firstErrorCode()`,
`firstErrorCodeOrNull()`,
`fieldMessages(...)`, `fieldHasErrors(...)`, `fieldHasMessages(...)`,
`fieldHasDetails(...)`, `fieldHasCodes(...)`, `fieldHasDetailWithCode(...)`,
`fieldHasMessageWithCode(...)`, `fieldState(...)`, `fieldStates(...)`,
`invalidFieldCount(...)`, `hasInvalidFields(...)`, `invalidFields(...)`, `firstInvalidField(...)`,
`firstInvalidFieldOrNull(...)`, `invalidFieldStates(...)`,
`firstInvalidFieldState(...)`, `firstInvalidFieldStateOrNull(...)`,
`fieldStateCountWithErrorCode(...)`, `hasFieldStateWithErrorCode(...)`,
`fieldStatesWithErrorCode(...)`, `firstFieldStateWithErrorCode(...)`,
`firstFieldStateWithErrorCodeOrNull(...)`,
`firstError(...)`, `firstErrorOrNull(...)`, `fieldMessageCount(...)`, `fieldMessageCountWithCode(...)`,
`fieldDetails(...)`, `fieldDetailCount(...)`,
`fieldDetailsWithCode(...)`, `fieldDetailCountWithCode(...)`,
`fieldHasDetailWithCode(...)`, `fieldMessagesWithCode(...)`,
`fieldMessageCountWithCode(...)`, `fieldHasMessageWithCode(...)`,
`fieldCodes(...)`, `fieldCodeCount(...)`, `fieldHasCodes(...)`,
`fieldHasErrorCode(...)`, `firstFieldMessage(...)`, `firstFieldMessageOrNull(...)`,
`firstFieldMessageWithCode(...)`, `firstFieldMessageWithCodeOrNull(...)`,
`firstFieldDetail(...)`, `firstFieldDetailOrNull(...)`,
`firstFieldDetailWithCode(...)`, `firstFieldDetailWithCodeOrNull(...)`,
and `firstFieldCode(...)` / `firstFieldCodeOrNull(...)`
reads for caught client-side validation throws. The error owns cloned error bags,
so mutating a caught error does not mutate the endpoint that produced it.
`fieldCodes(...)` and `fieldState(...).codes` return first-seen unique rule
codes for that field; `foundryValidationFieldStateHasErrorCode(...)` checks a
single field-state row, and `fieldDetails(...)` remains the repeated-detail
source when a UI needs every backend validation failure.
Use `setField(...)` from input handlers when a form should update one generated
field path and clear that path's generated
messages without rebuilding the whole bag. Generated setters clone accepted
request/form values, so mutating an object after passing it into `setField(...)`,
`setData(...)`, `patchData(...)`, or `resetData(...)` does not mutate endpoint
state without another generated setter call. Use `setFieldAndValidate(...)` when
a change should update the DTO and immediately refresh that field's backend-owned
client rules, or `setFieldAndValidateDependents(...)` when sibling-field rules
should refresh fields affected by the changed value. Use `validateField(...)` on
blur when the value was already written elsewhere,
`touchFieldAndValidate(...)` when blur should mark the field touched and refresh
errors in one emitted state update, `touchFieldAndValidateDependents(...)` when
blur should also refresh sibling-rule dependents, `touchFields([...])` when a
step should mark a known subset as touched, `touchFieldsAndValidate([...])` when
that step should also refresh its validation state, and `validateFields([...])`
when a step should refresh a known subset without changing touched state.
Per-submit `transform(data)` callbacks run before generated client-side
validation, so request normalization such as trimming or lowercasing fields is
validated and serialized through the same backend-owned DTO contract. The
transform receives cloned endpoint data, its returned payload is cloned before
request preparation, and `onStart(data)` receives a cloned payload too; return a
transformed payload or call a generated setter for persistent endpoint changes.
`prepareSubmit(...)` returns the resolved method, URL, serialized body or
`FormData`, adapter params, merged headers, full
HTTP request config, and submit mode from that same backend-owned preparation
path without sending the request. `prepareSubmitOrNull(...)` returns `null`
instead of throwing when params/options cannot produce a request envelope. Submit
lifecycle callbacks `onStart(data)`,
`onSuccess(response)`, `onError(error)`, and `onFinish()` are typed by the same
route-specific submit-options alias, and `onStart(...)` / `onSuccess(...)`
receive cloned request/response payloads, so custom form stores can run side
effects around generated validation and submission without local
request/response type aliases.
`submit(...)` is the Laravel/Inertia-style alias for `submitForm(...)`; use
`submitResponse(...)` when a status-discriminated response envelope is required.
Root container request schemas such as `Array<Dto>` and `Collection<Dto>`
validate the selected field across items and update container-prefixed paths.
Nested DTO paths also work: `validateField("profile.name")` validates the child
rule only, `validateField("children.name")` refreshes that child field across
`each(nested)` items, and exact generated paths such as
`validateField("children[0].name")` or `validateField("[tenant].email")` target
one item. `FoundryKnownFieldPath<T>` preserves known nested and indexed DTO
paths while endpoint helpers still accept runtime-only backend paths. Invalid
array or collection indexes are ignored instead of refreshing every item, while
string map keys remain supported through root map paths such as `[tenant].email`.
Nested root containers use the same prefix family, so generated paths such as
`[tenant][0].email` and `items[0][1].email` still read, validate, and clear
through the inner field name `email`.
Endpoint
field reads use the same path-family matching, so `firstFieldMessage("children.name")`,
`firstError("children.name")`, `fieldCodes("children.name")`, and
`fieldHasErrorCode("children.name", "required")`, `fieldDetailsWithCode(...)`,
and `fieldMessagesWithCode(...)` include generated item errors such as
`children[0].name`; standalone error-bag reads can opt into root container
aggregation through `FoundryValidationFieldReadOptions.includeContainerPaths`.
Use
`clearFieldErrors(...)` for field subsets or `clearFieldError(...)` for one
field when the UI only needs to clear messages. Use
`clearErrors("email", "password")` / `clearError("email")` when a Laravel-style
adapter wants the same behavior without wrapping the generated endpoint.
Generated route helpers export `{RouteName}RequestField` aliases for reusable
field props and field-subset helpers, `{RouteName}FieldValue<Field>` for typed
field component values, and route-specific touched/error/validation bag,
schema, and validation field-state aliases for stores that mirror generated
endpoint state.
Use `setFieldErrors(...)` / `setFieldError(...)` when local client checks or
external widgets need to hydrate generated field errors without a parallel store.
Use `defaults(...)` after partial saves when the current generated data should
become the clean baseline, and `reset(...)` when a Laravel-style adapter should
restore all generated data or a variadic field subset from that baseline. Use
`resetAndClearErrors(...)` when an adapter expects the Inertia-style combined
name; it delegates to the same generated reset path.
`validateForm()`, `validateField(...)`, and `applyValidationResult(...)` clear
stale backend error envelopes before replacing generated field state, and clear
stale success/response state when the resulting endpoint has validation errors.
Endpoint submissions clear stale response/status at submit start, apply the same
validation-envelope reset before client validation, clear stale generated field
errors before sending a request, then repopulate them from the current client
validation result or server `422` response. Server-error application also clears
stale successful response state before hydrating the error envelope.
Use `applyServerError(error)` when a custom adapter, wizard step, or non-standard
submit path catches a backend error but still wants the generated endpoint to
hydrate `serverError`, typed `errorResponse`, typed `validationErrorResponse`,
`errors`, and `errorDetails` through the same path used by `submitForm()`.
Use `applyResponse(response)` for custom success paths that need to hydrate the
typed `rawResponse`, response body, status, success pulse, and stale-progress
cleanup without duplicating generated submit state. Response bodies/envelopes
are cloned as they enter endpoint state.
Use `clearProgress()` when custom upload UI should dismiss progress state
without resetting submit counters, response state, or errors.
Use `cancel(...)` as a Laravel/Inertia-style alias for `abortPending(...)` when
the adapter should cancel generated in-flight submits.
Endpoint instances and `state()` snapshots expose `hasResponse`,
`hasProgress`, `hasServerError`, `hasErrorResponse`, and
`hasValidationErrorResponse` booleans for the same response/error/progress
presence checks.
Import a route's generated `{RouteName}Validation` constant with those helpers
when a custom Vue/React form wants backend-owned client checks, Foundry 422
field-error normalization, and typed field-message/code reads without creating
a generated endpoint instance. Generated validation constants are frozen at
runtime; schema selector helpers clone returned rule/message/attribute metadata
for local derived state instead of mutating the backend-owned contract.
Field-read helpers accept both
`FoundryValidationResult` and `FoundryValidationClientError`; `validateForm()`
and endpoint server-error handling use the same runtime paths.
Register one validation schema per DTO; `types:export` and fallible OpenAPI
generation reject duplicate `TsValidation` names so generated contracts cannot
silently depend on inventory ordering. Repeated `.field("name", ...)` calls on the same
`TsValidationSchema` merge their rules in call order, which makes it safe to
compose manual metadata from small helpers. Repeated `.message(field, rule, ...)`
and `.attribute(field, ...)` calls replace the existing entry for that key so
the generated arrays stay deterministic. Exact duplicate rules and duplicate
rule values are ignored by the fluent builders while preserving first-seen
order.
Manual validation metadata names must be non-empty and trimmed: schema field
names, rule codes, rule parameter keys, message field/rule keys, and attribute
field keys are validated before TypeScript/OpenAPI export. Custom messages and
attribute labels must also contain visible text. Message and attribute fields
must match declared manual schema fields, and message rule names must match a
client-side rule reachable from that field, including rules nested under
`each(...)`. Client-checkable manual rules must include the params or values
their generated TypeScript runtime case needs; for example `min` needs `min`,
`between` needs `min` and `max`, and `required_keys` / `in_list` need at least
one value, and rules cannot include extra params, values, nested rules, or
schemas that their generated runtime case ignores. `each(...)` needs at least
one nested item rule, and `nested` needs a child validation schema. Numeric rule
params must also parse to the shape generated validation uses: length/count
params are non-negative integers, numeric comparison params are finite numbers,
`multiple_of` must be positive, and `uuid` versions must be integers from 1
through 8. Range rules such as `decimal`, `digits_between`, and `between` must
provide `min <= max`, and `regex` / `not_regex` patterns must be valid Rust
regexes. Client-checkable regex metadata must also be browser-compatible after
Foundry's Rust-to-JavaScript translation; mark Rust-only patterns with
`.server_only()`. Field-list rules such as `required_with_all`, `required_without_all`,
and `prohibits` need `values` for runtime checks and
`params.other` for messages; prefer the matching `TsValidationRule` helper
constructors so both stay aligned. Value-list rules such as `starts_with`,
`ends_with`, and `contains` must keep `params.value` aligned with
`values.join(", ")` when both are present. Cross-field `other` params and
field-list values must be non-empty, trimmed serde wire field names so generated
client validation reads the same sibling keys as the backend.

### Use in route handler

```rust
async fn create_user(
    Validated(payload): Validated<CreateUser>,
) -> impl IntoResponse {
    // payload is validated — safe to use
    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "email": payload.email })),
    )
}
```

### JSON-only validated requests

Use `JsonValidated<T>` when an endpoint should only accept JSON and should reject multipart bodies automatically:

```rust
#[derive(Deserialize, ApiSchema, Validate)]
pub struct CreateSession {
    #[validate(required, email)]
    pub email: String,
    #[validate(required, min(8))]
    pub password: String,
}

async fn login(
    JsonValidated(payload): JsonValidated<CreateSession>,
) -> impl IntoResponse {
    Json(MessageResponse::ok())
}
```

This is the common path for JSON DTOs. `Validated<T>` remains the mixed extractor for DTOs that intentionally support both JSON and multipart.

When `Validated<T>` receives `multipart/form-data`, the derive-generated extractor now keeps typed parsing intact:

- scalar fields keep the last text part and parse via `FromStr`
- `Vec<T>` and `Option<Vec<T>>` fields collect repeated text parts in request order
- `serde_json::Value` fields parse from JSON text
- missing fields with `#[serde(default)]` or `#[serde(default = "...")]` use the same defaults as JSON requests
- invalid present multipart values fail the request with `400 Bad Request` instead of silently defaulting
- missing non-`Option` text fields without serde defaults fail the request with `400 Bad Request`

That means derive-based multipart DTOs no longer need local workarounds for optional numbers, repeated text fields, or arbitrary JSON payload fragments.
If a consumer form intentionally omits a field, model that field as `Option<T>` / `Option<Vec<T>>` or give it an explicit serde default in the DTO.

`JsonValidated<T>` now resolves extractor-level request errors through the validation/i18n pipeline too:

- `validation.invalid_request_body`
- `validation.multipart_not_supported`

That lets apps translate invalid JSON and JSON-only multipart rejections without wrapping the extractor locally.

### Route with OpenAPI documentation

```rust
r.route_with_options("/users", post(create_user),
    HttpRouteOptions::new()
        .document(RouteDoc::new()
            .post()
            .summary("Create user")
            .tag("users")
            .request::<CreateUser>()
            .response::<UserResponse>(201)
            .validation_errors()));
```

### Error response format

Validation failures use the backend-owned `ValidationErrorResponse` contract.
Generated TypeScript exports the same type, and OpenAPI route docs can reference
it with `validation_errors()`.

```json
{
    "message": "Validation failed",
    "status": 422,
    "errors": [
        {
            "field": "email",
            "code": "required",
            "message": "email is required"
        },
        {
            "field": "password",
            "code": "min",
            "message": "password must be at least 8 characters"
        }
    ]
}
```

---

## Implemented Rule Surface

| Category | Count |
|----------|-------|
| Presence and prohibition | 17 |
| String content and shape | 18 |
| Numeric and digit rules | 16 |
| Boolean and acceptance | 5 |
| Format and temporal parsing | 15 |
| List, collection, map, and enum rules | 10 |
| Comparison and confirmation | 6 |
| Database (async) | 2 |
| **Total built-in field/collection validation features** | **89** |

This total is derived from the backend `FieldRule` implementation. It excludes
custom `.rule(...)` validators and file upload rules such as `image`,
`max_file_size`, `allowed_mimes`, and `allowed_extensions`, which are documented
separately above. The three chain modifiers, `.nullable()`, `.bail()`, and
`.with_message(...)`, are separate controls rather than validation rules.
