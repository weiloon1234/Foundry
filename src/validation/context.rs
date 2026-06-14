use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use crate::foundation::{AppContext, Error, Result};
use crate::support::sync::{read_unpoisoned, write_unpoisoned};
use crate::support::ValidationRuleId;
use crate::validation::types::ValidationError;

#[derive(Clone)]
pub struct RuleContext {
    app: AppContext,
    field: String,
}

impl RuleContext {
    pub fn new(app: AppContext, field: impl Into<String>) -> Self {
        Self {
            app,
            field: field.into(),
        }
    }

    pub fn app(&self) -> &AppContext {
        &self.app
    }

    pub fn field(&self) -> &str {
        &self.field
    }
}

#[async_trait]
pub trait ValidationRule: Send + Sync + 'static {
    async fn validate(
        &self,
        context: &RuleContext,
        value: &str,
    ) -> std::result::Result<(), ValidationError>;
}

#[derive(Clone, Default)]
pub struct RuleRegistry {
    rules: Arc<RwLock<HashMap<ValidationRuleId, Arc<dyn ValidationRule>>>>,
}

impl RuleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<I>(&self, id: I, rule: impl ValidationRule) -> Result<()>
    where
        I: Into<ValidationRuleId>,
    {
        self.register_arc(id, Arc::new(rule))
    }

    pub fn register_arc<I>(&self, id: I, rule: Arc<dyn ValidationRule>) -> Result<()>
    where
        I: Into<ValidationRuleId>,
    {
        let id = id.into();
        let mut rules = write_unpoisoned(&self.rules, "rule registry");
        if rules.contains_key(&id) {
            return Err(Error::message(format!(
                "validation rule `{id}` already registered"
            )));
        }
        rules.insert(id, rule);
        Ok(())
    }

    pub fn get(&self, id: &ValidationRuleId) -> Result<Option<Arc<dyn ValidationRule>>> {
        Ok(read_unpoisoned(&self.rules, "rule registry")
            .get(id)
            .cloned())
    }
}
