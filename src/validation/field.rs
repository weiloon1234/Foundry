use crate::foundation::Result;
use crate::support::ValidationRuleId;
use crate::validation::executor::execute_steps;
use crate::validation::rules::{impl_field_rules, FieldRule, FieldStep};
use crate::validation::validator::Validator;

pub struct FieldValidator<'a> {
    pub(crate) validator: &'a mut Validator,
    pub(crate) field: String,
    pub(crate) value: String,
    pub(crate) present: bool,
    pub(crate) steps: Vec<FieldStep>,
    pub(crate) nullable: bool,
    pub(crate) sometimes: bool,
    pub(crate) bail: bool,
}

impl<'a> FieldValidator<'a> {
    impl_field_rules!();

    pub async fn apply(self) -> Result<()> {
        let FieldValidator {
            validator,
            field,
            value,
            present,
            steps,
            nullable,
            sometimes,
            bail,
        } = self;

        if sometimes && !present {
            return Ok(());
        }

        execute_steps(validator, &field, &value, present, steps, nullable, bail).await
    }
}

pub struct EachValidator<'a, T: AsRef<str>> {
    pub(crate) validator: &'a mut Validator,
    pub(crate) field: String,
    pub(crate) items: &'a [T],
    pub(crate) steps: Vec<FieldStep>,
    pub(crate) nullable: bool,
    pub(crate) sometimes: bool,
    pub(crate) bail: bool,
}

impl<'a, T: AsRef<str>> EachValidator<'a, T> {
    impl_field_rules!();

    pub async fn apply(self) -> Result<()> {
        let mut item_steps = Vec::with_capacity(self.steps.len());
        let mut distinct_steps = Vec::new();
        for step in self.steps {
            if matches!(&step.rule, FieldRule::Distinct) {
                distinct_steps.push(step);
            } else {
                item_steps.push(step);
            }
        }

        for step in distinct_steps {
            let mut seen = std::collections::HashSet::with_capacity(self.items.len());
            let has_duplicate = self.items.iter().any(|item| !seen.insert(item.as_ref()));
            if has_duplicate {
                let message = self.validator.resolve_message(
                    &self.field,
                    "distinct",
                    &[],
                    step.message.as_deref(),
                );
                self.validator.push_error(
                    self.field.clone(),
                    crate::validation::ValidationError::new("distinct", message),
                );
                if self.bail {
                    return Ok(());
                }
            }
        }

        for (i, item) in self.items.iter().enumerate() {
            let item_field = format!("{}[{}]", self.field, i);
            let value = item.as_ref();
            execute_steps(
                self.validator,
                &item_field,
                value,
                true,
                item_steps.clone(),
                self.nullable,
                self.bail,
            )
            .await?;
        }
        Ok(())
    }
}
