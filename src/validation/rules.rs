use crate::support::ValidationRuleId;

#[derive(Clone)]
pub(crate) enum FieldRule {
    Required,
    RequiredIf {
        other_field: String,
        other_value: String,
        expected_values: Vec<String>,
    },
    RequiredUnless {
        other_field: String,
        other_value: String,
        expected_values: Vec<String>,
    },
    RequiredWith {
        other_fields: Vec<(String, String)>,
    },
    Present,
    Prohibited,
    Email,
    Min(usize),
    Max(usize),
    Named(ValidationRuleId),
    Regex(String),
    Url,
    Uuid,
    Numeric,
    Boolean,
    Alpha,
    AlphaNumeric,
    InList(Vec<String>),
    NotIn(Vec<String>),
    StartsWith(String),
    EndsWith(String),
    Ip,
    Json,
    Confirmed {
        other_field: String,
        other_value: String,
    },
    Digits,
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
    MinNumeric(f64),
    MaxNumeric(f64),
    Integer,
    Between {
        min: f64,
        max: f64,
    },
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
    Distinct,
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

        pub fn required_if(
            mut self,
            other_field: impl Into<String>,
            other_value: impl Into<String>,
            expected_values: impl IntoIterator<Item = impl Into<String>>,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::RequiredIf {
                    other_field: other_field.into(),
                    other_value: other_value.into(),
                    expected_values: expected_values.into_iter().map(Into::into).collect(),
                },
                message: None,
            });
            self
        }

        pub fn required_unless(
            mut self,
            other_field: impl Into<String>,
            other_value: impl Into<String>,
            expected_values: impl IntoIterator<Item = impl Into<String>>,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::RequiredUnless {
                    other_field: other_field.into(),
                    other_value: other_value.into(),
                    expected_values: expected_values.into_iter().map(Into::into).collect(),
                },
                message: None,
            });
            self
        }

        pub fn required_with<I, N, V>(mut self, other_fields: I) -> Self
        where
            I: IntoIterator<Item = (N, V)>,
            N: Into<String>,
            V: Into<String>,
        {
            self.steps.push(FieldStep {
                rule: FieldRule::RequiredWith {
                    other_fields: other_fields
                        .into_iter()
                        .map(|(name, value)| (name.into(), value.into()))
                        .collect(),
                },
                message: None,
            });
            self
        }

        pub fn present(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Present,
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

        pub fn max(mut self, length: usize) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Max(length),
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

        pub fn url(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Url,
                message: None,
            });
            self
        }

        pub fn uuid(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Uuid,
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

        pub fn boolean(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Boolean,
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

        pub fn alpha_numeric(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::AlphaNumeric,
                message: None,
            });
            self
        }

        pub fn in_list(mut self, values: impl IntoIterator<Item = impl Into<String>>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::InList(values.into_iter().map(Into::into).collect()),
                message: None,
            });
            self
        }

        pub fn not_in(mut self, values: impl IntoIterator<Item = impl Into<String>>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::NotIn(values.into_iter().map(Into::into).collect()),
                message: None,
            });
            self
        }

        pub fn starts_with(mut self, prefix: impl Into<String>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::StartsWith(prefix.into()),
                message: None,
            });
            self
        }

        pub fn ends_with(mut self, suffix: impl Into<String>) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::EndsWith(suffix.into()),
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

        pub fn confirmed(
            mut self,
            other_field: impl Into<String>,
            other_value: impl Into<String>,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Confirmed {
                    other_field: other_field.into(),
                    other_value: other_value.into(),
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
            other_value: impl Into<String>,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Before {
                    other_field: other_field.into(),
                    other_value: other_value.into(),
                    allow_equal: false,
                },
                message: None,
            });
            self
        }

        pub fn before_or_equal(
            mut self,
            other_field: impl Into<String>,
            other_value: impl Into<String>,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Before {
                    other_field: other_field.into(),
                    other_value: other_value.into(),
                    allow_equal: true,
                },
                message: None,
            });
            self
        }

        pub fn after(
            mut self,
            other_field: impl Into<String>,
            other_value: impl Into<String>,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::After {
                    other_field: other_field.into(),
                    other_value: other_value.into(),
                    allow_equal: false,
                },
                message: None,
            });
            self
        }

        pub fn after_or_equal(
            mut self,
            other_field: impl Into<String>,
            other_value: impl Into<String>,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::After {
                    other_field: other_field.into(),
                    other_value: other_value.into(),
                    allow_equal: true,
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

        pub fn same(
            mut self,
            other_field: impl Into<String>,
            other_value: impl Into<String>,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Same {
                    other_field: other_field.into(),
                    other_value: other_value.into(),
                },
                message: None,
            });
            self
        }

        pub fn different(
            mut self,
            other_field: impl Into<String>,
            other_value: impl Into<String>,
        ) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Different {
                    other_field: other_field.into(),
                    other_value: other_value.into(),
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
            let keys: Vec<String> = E::keys()
                .into_iter()
                .map(|k| match k {
                    ::foundry::app_enum::EnumKey::String(s) => s,
                    ::foundry::app_enum::EnumKey::Int(n) => n.to_string(),
                })
                .collect();
            self.steps.push(FieldStep {
                rule: FieldRule::AppEnum { valid_keys: keys },
                message: None,
            });
            self
        }

        pub fn distinct(mut self) -> Self {
            self.steps.push(FieldStep {
                rule: FieldRule::Distinct,
                message: None,
            });
            self
        }

        pub fn nullable(mut self) -> Self {
            self.nullable = true;
            self
        }

        pub fn sometimes(mut self) -> Self {
            self.sometimes = true;
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
