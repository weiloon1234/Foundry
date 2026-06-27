use crate::support::ValidationRuleId;

#[derive(Clone)]
pub(crate) enum FieldRule {
    Required,
    Filled,
    FilledCollection,
    Email,
    Min(usize),
    Max(usize),
    Size(usize),
    SizeNumeric(f64),
    Named(ValidationRuleId),
    Regex(String),
    NotRegex(String),
    Url,
    Uuid {
        version: Option<u8>,
    },
    Ulid,
    HexColor,
    MacAddress,
    Numeric,
    Decimal {
        min: usize,
        max: usize,
    },
    Boolean,
    Accepted,
    AcceptedIf {
        other_field: String,
        other_value: String,
        expected_value: String,
    },
    Declined,
    DeclinedIf {
        other_field: String,
        other_value: String,
        expected_value: String,
    },
    Alpha,
    AlphaDash,
    AlphaNum,
    AlphaNumeric,
    Ascii,
    Lowercase,
    Uppercase,
    InList(Vec<String>),
    NotIn(Vec<String>),
    StartsWith(Vec<String>),
    DoesntStartWith(Vec<String>),
    EndsWith(Vec<String>),
    DoesntEndWith(Vec<String>),
    Contains(String),
    DoesntContain(String),
    ContainsAll(Vec<String>),
    DoesntContainAny(Vec<String>),
    MinItems(usize),
    MaxItems(usize),
    SizeItems(usize),
    Distinct,
    Ip,
    Json,
    RequiredIf {
        other_field: String,
        other_value: String,
        expected_value: String,
    },
    RequiredUnless {
        other_field: String,
        other_value: String,
        except_value: String,
    },
    RequiredIfAccepted {
        other_field: String,
        other_value: String,
    },
    RequiredIfDeclined {
        other_field: String,
        other_value: String,
    },
    Prohibited,
    ProhibitedIf {
        other_field: String,
        other_value: String,
        expected_value: String,
    },
    ProhibitedUnless {
        other_field: String,
        other_value: String,
        except_value: String,
    },
    ProhibitedIfAccepted {
        other_field: String,
        other_value: String,
    },
    ProhibitedIfDeclined {
        other_field: String,
        other_value: String,
    },
    Prohibits {
        other_fields: Vec<(String, String)>,
    },
    RequiredWith {
        other_field: String,
        other_value: String,
    },
    RequiredWithAll {
        other_fields: Vec<(String, String)>,
    },
    RequiredWithout {
        other_field: String,
        other_value: String,
    },
    RequiredWithoutAll {
        other_fields: Vec<(String, String)>,
    },
    #[allow(dead_code)]
    RequiredKeys(Vec<String>),
    Confirmed {
        other_field: String,
        other_value: String,
    },
    Digits,
    MinDigits(usize),
    MaxDigits(usize),
    DigitsBetween {
        min: usize,
        max: usize,
    },
    Timezone,
    Date,
    Time,
    DateTime,
    LocalDateTime,
    Before {
        other_field: String,
        other_value: String,
        allow_equal: bool,
    },
    After {
        other_field: String,
        other_value: String,
        allow_equal: bool,
    },
    DateEquals {
        other_field: String,
        other_value: String,
    },
    MinNumeric(f64),
    MaxNumeric(f64),
    MultipleOf(f64),
    Integer,
    Between {
        min: f64,
        max: f64,
    },
    Gt(f64),
    Gte(f64),
    Lt(f64),
    Lte(f64),
    Ipv4,
    Ipv6,
    Same {
        other_field: String,
        other_value: String,
    },
    Different {
        other_field: String,
        other_value: String,
    },
    Unique {
        table: String,
        column: String,
    },
    Exists {
        table: String,
        column: String,
    },
    AppEnum {
        valid_keys: Vec<String>,
    },
}

#[derive(Clone)]
pub(crate) struct FieldStep {
    pub rule: FieldRule,
    pub message: Option<String>,
}

macro_rules! impl_field_rules {
    () => {
        pub fn required(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Required,
                message: None,
            });
            self
        }

        pub fn filled(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Filled,
                message: None,
            });
            self
        }

        pub fn email(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Email,
                message: None,
            });
            self
        }

        pub fn min(mut self, length: usize) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Min(length),
                message: None,
            });
            self
        }

        pub fn min_length(self, length: usize) -> Self {
            self.min(length)
        }

        pub fn max(mut self, length: usize) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Max(length),
                message: None,
            });
            self
        }

        pub fn max_length(self, length: usize) -> Self {
            self.max(length)
        }

        pub fn size(mut self, length: usize) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Size(length),
                message: None,
            });
            self
        }

        pub fn size_numeric(mut self, value: f64) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::SizeNumeric(value),
                message: None,
            });
            self
        }

        pub fn rule<I>(mut self, id: I) -> Self
        where
            I: Into<ValidationRuleId>,
        {
            self.steps.push(FieldStep {
                rule: FieldRule::Named(id.into()),
                message: None,
            });
            self
        }

        pub fn regex(mut self, pattern: impl Into<String>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Regex(pattern.into()),
                message: None,
            });
            self
        }

        pub fn not_regex(mut self, pattern: impl Into<String>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::NotRegex(pattern.into()),
                message: None,
            });
            self
        }

        pub fn url(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Url,
                message: None,
            });
            self
        }

        pub fn uuid(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Uuid { version: None },
                message: None,
            });
            self
        }

        pub fn uuid_version(mut self, version: u8) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Uuid {
                    version: Some(version),
                },
                message: None,
            });
            self
        }

        pub fn ulid(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Ulid,
                message: None,
            });
            self
        }

        pub fn hex_color(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::HexColor,
                message: None,
            });
            self
        }

        pub fn mac_address(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::MacAddress,
                message: None,
            });
            self
        }

        pub fn numeric(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Numeric,
                message: None,
            });
            self
        }

        pub fn decimal(mut self, min: usize, max: usize) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Decimal { min, max },
                message: None,
            });
            self
        }

        pub fn boolean(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Boolean,
                message: None,
            });
            self
        }

        pub fn accepted(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Accepted,
                message: None,
            });
            self
        }

        pub fn accepted_if(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
            expected_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::AcceptedIf {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                    expected_value: expected_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn declined(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Declined,
                message: None,
            });
            self
        }

        pub fn declined_if(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
            expected_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::DeclinedIf {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                    expected_value: expected_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn alpha(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Alpha,
                message: None,
            });
            self
        }

        pub fn alpha_dash(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::AlphaDash,
                message: None,
            });
            self
        }

        pub fn alpha_num(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::AlphaNum,
                message: None,
            });
            self
        }

        pub fn alpha_numeric(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::AlphaNumeric,
                message: None,
            });
            self
        }

        pub fn ascii(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Ascii,
                message: None,
            });
            self
        }

        pub fn lowercase(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Lowercase,
                message: None,
            });
            self
        }

        pub fn uppercase(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Uppercase,
                message: None,
            });
            self
        }

        pub fn in_list(mut self, values: impl IntoIterator<Item = impl ToString>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::InList(
                    values.into_iter().map(|value| value.to_string()).collect(),
                ),
                message: None,
            });
            self
        }

        pub fn not_in(mut self, values: impl IntoIterator<Item = impl ToString>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::NotIn(values.into_iter().map(|value| value.to_string()).collect()),
                message: None,
            });
            self
        }

        pub fn starts_with(mut self, prefix: impl Into<String>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::StartsWith(vec![prefix.into()]),
                message: None,
            });
            self
        }

        pub fn starts_with_any(
            mut self,
            prefixes: impl IntoIterator<Item = impl ToString>,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::StartsWith(
                    prefixes
                        .into_iter()
                        .map(|prefix| prefix.to_string())
                        .collect(),
                ),
                message: None,
            });
            self
        }

        pub fn doesnt_start_with(mut self, prefix: impl Into<String>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::DoesntStartWith(vec![prefix.into()]),
                message: None,
            });
            self
        }

        pub fn doesnt_start_with_any(
            mut self,
            prefixes: impl IntoIterator<Item = impl ToString>,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::DoesntStartWith(
                    prefixes
                        .into_iter()
                        .map(|prefix| prefix.to_string())
                        .collect(),
                ),
                message: None,
            });
            self
        }

        pub fn ends_with(mut self, suffix: impl Into<String>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::EndsWith(vec![suffix.into()]),
                message: None,
            });
            self
        }

        pub fn ends_with_any(mut self, suffixes: impl IntoIterator<Item = impl ToString>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::EndsWith(
                    suffixes
                        .into_iter()
                        .map(|suffix| suffix.to_string())
                        .collect(),
                ),
                message: None,
            });
            self
        }

        pub fn doesnt_end_with(mut self, suffix: impl Into<String>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::DoesntEndWith(vec![suffix.into()]),
                message: None,
            });
            self
        }

        pub fn doesnt_end_with_any(
            mut self,
            suffixes: impl IntoIterator<Item = impl ToString>,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::DoesntEndWith(
                    suffixes
                        .into_iter()
                        .map(|suffix| suffix.to_string())
                        .collect(),
                ),
                message: None,
            });
            self
        }

        pub fn contains(mut self, needle: impl Into<String>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Contains(needle.into()),
                message: None,
            });
            self
        }

        pub fn doesnt_contain(mut self, needle: impl Into<String>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::DoesntContain(needle.into()),
                message: None,
            });
            self
        }

        pub fn ip(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Ip,
                message: None,
            });
            self
        }

        pub fn json(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Json,
                message: None,
            });
            self
        }

        pub fn required_if(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
            expected_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::RequiredIf {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                    expected_value: expected_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn required_unless(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
            except_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::RequiredUnless {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                    except_value: except_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn required_if_accepted(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::RequiredIfAccepted {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn required_if_declined(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::RequiredIfDeclined {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn prohibited(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Prohibited,
                message: None,
            });
            self
        }

        pub fn prohibited_if(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
            expected_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::ProhibitedIf {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                    expected_value: expected_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn prohibited_unless(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
            except_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::ProhibitedUnless {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                    except_value: except_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn prohibited_if_accepted(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::ProhibitedIfAccepted {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn prohibited_if_declined(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::ProhibitedIfDeclined {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn prohibits<I, K, V>(mut self, fields: I) -> Self
        where
            I: IntoIterator<Item = (K, V)>,
            K: Into<String>,
            V: ToString,
        {
            self.steps.push(FieldStep {
                rule: FieldRule::Prohibits {
                    other_fields: fields
                        .into_iter()
                        .map(|(field, value)| (field.into(), value.to_string()))
                        .collect(),
                },
                message: None,
            });
            self
        }

        pub fn required_with(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::RequiredWith {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn required_with_all<I, K, V>(mut self, fields: I) -> Self
        where
            I: IntoIterator<Item = (K, V)>,
            K: Into<String>,
            V: ToString,
        {
            self.steps.push(FieldStep {
                rule: FieldRule::RequiredWithAll {
                    other_fields: fields
                        .into_iter()
                        .map(|(field, value)| (field.into(), value.to_string()))
                        .collect(),
                },
                message: None,
            });
            self
        }

        pub fn required_without(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::RequiredWithout {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn required_without_all<I, K, V>(mut self, fields: I) -> Self
        where
            I: IntoIterator<Item = (K, V)>,
            K: Into<String>,
            V: ToString,
        {
            self.steps.push(FieldStep {
                rule: FieldRule::RequiredWithoutAll {
                    other_fields: fields
                        .into_iter()
                        .map(|(field, value)| (field.into(), value.to_string()))
                        .collect(),
                },
                message: None,
            });
            self
        }

        pub fn confirmed(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Confirmed {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn digits(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Digits,
                message: None,
            });
            self
        }

        pub fn min_digits(mut self, min: usize) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::MinDigits(min),
                message: None,
            });
            self
        }

        pub fn max_digits(mut self, max: usize) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::MaxDigits(max),
                message: None,
            });
            self
        }

        pub fn digits_between(mut self, min: usize, max: usize) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::DigitsBetween { min, max },
                message: None,
            });
            self
        }

        pub fn timezone(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Timezone,
                message: None,
            });
            self
        }

        pub fn date(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Date,
                message: None,
            });
            self
        }

        pub fn time(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Time,
                message: None,
            });
            self
        }

        pub fn datetime(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::DateTime,
                message: None,
            });
            self
        }

        pub fn local_datetime(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::LocalDateTime,
                message: None,
            });
            self
        }

        pub fn before(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Before {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                    allow_equal: false,
                },
                message: None,
            });
            self
        }

        pub fn before_or_equal(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Before {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                    allow_equal: true,
                },
                message: None,
            });
            self
        }

        pub fn after(mut self, other_field: impl Into<String>, other_value: impl ToString) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::After {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                    allow_equal: false,
                },
                message: None,
            });
            self
        }

        pub fn after_or_equal(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::After {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                    allow_equal: true,
                },
                message: None,
            });
            self
        }

        pub fn date_equals(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::DateEquals {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn min_numeric(mut self, min: f64) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::MinNumeric(min),
                message: None,
            });
            self
        }

        pub fn max_numeric(mut self, max: f64) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::MaxNumeric(max),
                message: None,
            });
            self
        }

        pub fn multiple_of(mut self, value: f64) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::MultipleOf(value),
                message: None,
            });
            self
        }

        pub fn integer(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Integer,
                message: None,
            });
            self
        }

        pub fn between(mut self, min: f64, max: f64) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Between { min, max },
                message: None,
            });
            self
        }

        pub fn gt(mut self, value: f64) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Gt(value),
                message: None,
            });
            self
        }

        pub fn gte(mut self, value: f64) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Gte(value),
                message: None,
            });
            self
        }

        pub fn lt(mut self, value: f64) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Lt(value),
                message: None,
            });
            self
        }

        pub fn lte(mut self, value: f64) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Lte(value),
                message: None,
            });
            self
        }

        pub fn ipv4(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Ipv4,
                message: None,
            });
            self
        }

        pub fn ipv6(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Ipv6,
                message: None,
            });
            self
        }

        pub fn same(mut self, other_field: impl Into<String>, other_value: impl ToString) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Same {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn different(
            mut self,
            other_field: impl Into<String>,
            other_value: impl ToString,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Different {
                    other_field: other_field.into(),
                    other_value: other_value.to_string(),
                },
                message: None,
            });
            self
        }

        pub fn unique(mut self, table: impl Into<String>, column: impl Into<String>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Unique {
                    table: table.into(),
                    column: column.into(),
                },
                message: None,
            });
            self
        }

        pub fn exists(mut self, table: impl Into<String>, column: impl Into<String>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Exists {
                    table: table.into(),
                    column: column.into(),
                },
                message: None,
            });
            self
        }

        pub fn app_enum<E: ::foundry::app_enum::FoundryAppEnum>(mut self) -> Self {
            let keys = E::accepted_keys().into_vec();
            self.steps.push(FieldStep {
                rule: FieldRule::AppEnum { valid_keys: keys },
                message: None,
            });
            self
        }

        pub fn nullable(mut self) -> Self {
            self.nullable = true;
            self
        }

        pub fn bail(mut self) -> Self {
            self.bail = true;
            self
        }

        pub fn with_message(mut self, message: impl Into<String>) -> Self {
            if let Some(last) = self.steps.last_mut() {
                last.message = Some(message.into());
            }
            self
        }
    };
}

pub(crate) use impl_field_rules;
