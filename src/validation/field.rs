use std::collections::HashSet;

use crate::foundation::Result;
use crate::support::ValidationRuleId;
use crate::validation::executor::execute_steps;
use crate::validation::rules::{impl_field_rules, FieldRule, FieldStep};
use crate::validation::types::ValidationError;
use crate::validation::validator::Validator;

pub struct FieldValidator<'a> {
    pub(crate) validator: &'a mut Validator,
    pub(crate) field: String,
    pub(crate) value: String,
    pub(crate) steps: Vec<FieldStep>,
    pub(crate) nullable: bool,
    pub(crate) bail: bool,
}

impl<'a> FieldValidator<'a> {
    impl_field_rules!();

    pub async fn apply(self) -> Result<()> {
        let FieldValidator {
            validator,
            field,
            value,
            steps,
            nullable,
            bail,
        } = self;

        // Nullable: skip all rules if value is empty
        if nullable && value.trim().is_empty() {
            return Ok(());
        }

        execute_steps(validator, &field, &value, steps, bail).await
    }
}

pub struct KeyValidator<'a> {
    pub(crate) validator: &'a mut Validator,
    pub(crate) field: String,
    pub(crate) keys: Option<Vec<String>>,
    pub(crate) required_keys: Vec<String>,
    pub(crate) message: Option<String>,
}

impl<'a> KeyValidator<'a> {
    pub fn required_keys(mut self, keys: impl IntoIterator<Item = impl ToString>) -> Self {
        self.required_keys
            .extend(keys.into_iter().map(|key| key.to_string()));
        self
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub async fn apply(self) -> Result<()> {
        if self.required_keys.is_empty() {
            return Ok(());
        }

        let present_keys = self
            .keys
            .as_ref()
            .map(|keys| keys.iter().map(String::as_str).collect::<HashSet<_>>());
        let missing_keys = self
            .required_keys
            .iter()
            .filter(|key| {
                present_keys
                    .as_ref()
                    .is_none_or(|keys| !keys.contains(key.as_str()))
            })
            .map(String::as_str)
            .collect::<Vec<_>>();

        if missing_keys.is_empty() {
            return Ok(());
        }

        let keys = missing_keys.join(", ");
        let msg = self.validator.resolve_message(
            &self.field,
            "required_keys",
            &[("keys", &keys)],
            self.message.as_deref(),
        );
        self.validator
            .push_error(self.field, ValidationError::new("required_keys", msg));
        Ok(())
    }
}

pub struct EachValidator<'a, T: ToString> {
    pub(crate) validator: &'a mut Validator,
    pub(crate) field: String,
    pub(crate) items: &'a [T],
    pub(crate) steps: Vec<FieldStep>,
    pub(crate) nullable: bool,
    pub(crate) bail: bool,
}

impl<'a, T: ToString> EachValidator<'a, T> {
    impl_field_rules!();

    #[doc(hidden)]
    pub fn filled_collection(mut self) -> Self {
        self.steps.push(FieldStep {
            rule: FieldRule::FilledCollection,
            message: None,
        });
        self
    }

    pub fn min_items(mut self, min: usize) -> Self {
        self.steps.push(FieldStep {
            rule: FieldRule::MinItems(min),
            message: None,
        });
        self
    }

    pub fn max_items(mut self, max: usize) -> Self {
        self.steps.push(FieldStep {
            rule: FieldRule::MaxItems(max),
            message: None,
        });
        self
    }

    pub fn size_items(mut self, size: usize) -> Self {
        self.steps.push(FieldStep {
            rule: FieldRule::SizeItems(size),
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

    pub fn contains_all(mut self, values: impl IntoIterator<Item = impl ToString>) -> Self {
        self.steps.push(FieldStep {
            rule: FieldRule::ContainsAll(
                values.into_iter().map(|value| value.to_string()).collect(),
            ),
            message: None,
        });
        self
    }

    pub fn doesnt_contain_any(mut self, values: impl IntoIterator<Item = impl ToString>) -> Self {
        self.steps.push(FieldStep {
            rule: FieldRule::DoesntContainAny(
                values.into_iter().map(|value| value.to_string()).collect(),
            ),
            message: None,
        });
        self
    }

    pub async fn apply(self) -> Result<()> {
        let mut item_steps = Vec::new();
        let mut collection_steps = Vec::new();
        for step in self.steps {
            if matches!(
                &step.rule,
                FieldRule::FilledCollection
                    | FieldRule::MinItems(_)
                    | FieldRule::MaxItems(_)
                    | FieldRule::SizeItems(_)
                    | FieldRule::Distinct
                    | FieldRule::ContainsAll(_)
                    | FieldRule::DoesntContainAny(_)
            ) {
                collection_steps.push(step);
            } else {
                item_steps.push(step);
            }
        }

        let item_values = self
            .items
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        for step in collection_steps {
            let errors_before = self.validator.errors.len();
            match step {
                FieldStep {
                    rule: FieldRule::FilledCollection,
                    message,
                } => {
                    if self.items.is_empty() {
                        let msg = self.validator.resolve_message(
                            &self.field,
                            "filled",
                            &[],
                            message.as_deref(),
                        );
                        self.validator
                            .push_error(self.field.clone(), ValidationError::new("filled", msg));
                    }
                }
                FieldStep {
                    rule: FieldRule::MinItems(min),
                    message,
                } => {
                    if self.items.len() < min {
                        let msg = self.validator.resolve_message(
                            &self.field,
                            "min_items",
                            &[("min", &min.to_string())],
                            message.as_deref(),
                        );
                        self.validator
                            .push_error(self.field.clone(), ValidationError::new("min_items", msg));
                    }
                }
                FieldStep {
                    rule: FieldRule::MaxItems(max),
                    message,
                } => {
                    if self.items.len() > max {
                        let msg = self.validator.resolve_message(
                            &self.field,
                            "max_items",
                            &[("max", &max.to_string())],
                            message.as_deref(),
                        );
                        self.validator
                            .push_error(self.field.clone(), ValidationError::new("max_items", msg));
                    }
                }
                FieldStep {
                    rule: FieldRule::SizeItems(size),
                    message,
                } => {
                    if self.items.len() != size {
                        let msg = self.validator.resolve_message(
                            &self.field,
                            "size",
                            &[("size", &size.to_string())],
                            message.as_deref(),
                        );
                        self.validator
                            .push_error(self.field.clone(), ValidationError::new("size", msg));
                    }
                }
                FieldStep {
                    rule: FieldRule::Distinct,
                    message,
                } => {
                    let mut seen = HashSet::new();
                    if item_values.iter().any(|item| !seen.insert(item)) {
                        let msg = self.validator.resolve_message(
                            &self.field,
                            "distinct",
                            &[],
                            message.as_deref(),
                        );
                        self.validator
                            .push_error(self.field.clone(), ValidationError::new("distinct", msg));
                    }
                }
                FieldStep {
                    rule: FieldRule::ContainsAll(values),
                    message,
                } => {
                    if !values.iter().all(|required| item_values.contains(required)) {
                        let values = values.join(", ");
                        let msg = self.validator.resolve_message(
                            &self.field,
                            "contains",
                            &[("value", &values)],
                            message.as_deref(),
                        );
                        self.validator
                            .push_error(self.field.clone(), ValidationError::new("contains", msg));
                    }
                }
                FieldStep {
                    rule: FieldRule::DoesntContainAny(values),
                    message,
                } => {
                    if values
                        .iter()
                        .any(|forbidden| item_values.contains(forbidden))
                    {
                        let values = values.join(", ");
                        let msg = self.validator.resolve_message(
                            &self.field,
                            "doesnt_contain",
                            &[("value", &values)],
                            message.as_deref(),
                        );
                        self.validator.push_error(
                            self.field.clone(),
                            ValidationError::new("doesnt_contain", msg),
                        );
                    }
                }
                _ => unreachable!(),
            }

            if self.bail && self.validator.errors.len() > errors_before {
                return Ok(());
            }
        }

        for (i, item) in self.items.iter().enumerate() {
            let item_field = format!("{}[{}]", self.field, i);
            let value = item.to_string();
            if self.nullable && value.trim().is_empty() {
                continue;
            }
            execute_steps(
                self.validator,
                &item_field,
                &value,
                item_steps.clone(),
                self.bail,
            )
            .await?;
        }
        Ok(())
    }
}
