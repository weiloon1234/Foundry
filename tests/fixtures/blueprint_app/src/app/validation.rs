use async_trait::async_trait;
use foundry::prelude::*;

pub struct MobileRule;

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
