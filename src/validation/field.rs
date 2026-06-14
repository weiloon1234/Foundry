use crate::foundation::Result;
use crate::support::ValidationRuleId;
use crate::validation::executor::execute_steps;
use crate::validation::rules::{impl_field_rules, FieldRule, FieldStep};
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

pub struct EachValidator<'a, T: AsRef<str>> {
    pub(crate) validator: &'a mut Validator,
    pub(crate) field: String,
    pub(crate) items: &'a [T],
    pub(crate) steps: Vec<FieldStep>,
    pub(crate) nullable: bool,
    pub(crate) bail: bool,
}

impl<'a, T: AsRef<str>> EachValidator<'a, T> {
    impl_field_rules!();

    pub async fn apply(self) -> Result<()> {
        for (i, item) in self.items.iter().enumerate() {
            let item_field = format!("{}[{}]", self.field, i);
            let value = item.as_ref();
            if self.nullable && value.trim().is_empty() {
                continue;
            }
            execute_steps(
                self.validator,
                &item_field,
                value,
                self.steps.clone(),
                self.bail,
            )
            .await?;
        }
        Ok(())
    }
}
