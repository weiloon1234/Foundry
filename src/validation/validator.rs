use std::collections::HashMap;

use crate::foundation::AppContext;
use crate::validation::executor::{fallback_message, interpolate_message};
use crate::validation::field::{EachValidator, FieldValidator};
use crate::validation::types::{FieldError, ValidationError, ValidationErrors};

pub struct Validator {
    pub(crate) app: AppContext,
    pub(crate) errors: Vec<FieldError>,
    pub(crate) locale: Option<String>,
    pub(crate) custom_messages: HashMap<(String, String), String>,
    pub(crate) custom_attributes: HashMap<String, String>,
}

impl Validator {
    pub fn new(app: AppContext) -> Self {
        Self {
            app,
            errors: Vec::new(),
            locale: None,
            custom_messages: HashMap::new(),
            custom_attributes: HashMap::new(),
        }
    }

    pub fn app(&self) -> &AppContext {
        &self.app
    }

    pub fn field<'a>(
        &'a mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> FieldValidator<'a> {
        FieldValidator {
            validator: self,
            field: name.into(),
            value: value.into(),
            steps: Vec::new(),
            nullable: false,
            bail: false,
        }
    }

    pub fn each<'a, T>(
        &'a mut self,
        field: impl Into<String>,
        items: &'a [T],
    ) -> EachValidator<'a, T>
    where
        T: AsRef<str>,
    {
        EachValidator {
            validator: self,
            field: field.into(),
            items,
            steps: Vec::new(),
            nullable: false,
            bail: false,
        }
    }

    pub fn finish(self) -> std::result::Result<(), ValidationErrors> {
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors::new(self.errors))
        }
    }

    pub(crate) fn push_error(&mut self, field: String, error: ValidationError) {
        self.errors.push(FieldError {
            field,
            code: error.code,
            message: error.message,
        });
    }

    /// Add a validation error for a field with automatic message resolution.
    ///
    /// Used by the Validate derive macro for file validation rules.
    pub fn add_error(&mut self, field: &str, code: &str, params: &[(&str, &str)]) {
        let msg = self.resolve_message(field, code, params, None);
        self.errors.push(FieldError {
            field: field.to_string(),
            code: code.to_string(),
            message: msg,
        });
    }

    pub fn locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = Some(locale.into());
        self
    }

    pub fn set_locale(&mut self, locale: impl Into<String>) {
        self.locale = Some(locale.into());
    }

    pub fn custom_message(
        &mut self,
        field: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.custom_messages
            .insert((field.into(), code.into()), message.into());
    }

    pub fn custom_attribute(&mut self, field: impl Into<String>, name: impl Into<String>) {
        self.custom_attributes.insert(field.into(), name.into());
    }

    pub(crate) fn resolve_field_attribute(&self, field: &str) -> String {
        let base_field = match field.find('[') {
            Some(pos) => &field[..pos],
            None => field,
        };

        // Priority 1: validator-level custom_attribute (exact match)
        if let Some(name) = self.custom_attributes.get(field) {
            return self.resolve_attribute_label(name);
        }
        // Priority 1b: validator-level custom_attribute (base field match)
        if base_field != field {
            if let Some(name) = self.custom_attributes.get(base_field) {
                return self.resolve_attribute_label(name);
            }
        }
        // Priority 2: i18n validation.attributes.{field}
        if let Ok(manager) = self.app.i18n() {
            let locale = self.locale.as_deref().unwrap_or(manager.default_locale());
            let key = format!("validation.attributes.{}", base_field);
            let resolved = manager.translate(locale, &key, &[]);
            if resolved != key {
                return resolved;
            }
        }
        // Priority 3: raw field name
        base_field.to_string()
    }

    fn resolve_attribute_label(&self, name: &str) -> String {
        if let Ok(manager) = self.app.i18n() {
            let locale = self.locale.as_deref().unwrap_or(manager.default_locale());
            let resolved = manager.translate(locale, name, &[]);
            if resolved != name {
                return resolved;
            }
        }

        name.to_string()
    }

    pub(crate) fn resolve_message(
        &self,
        field: &str,
        code: &str,
        params: &[(&str, &str)],
        custom_message: Option<&str>,
    ) -> String {
        let attribute = self.resolve_field_attribute(field);
        let mut all_params = vec![("attribute", attribute.as_str())];
        all_params.extend_from_slice(params);

        // Priority 1: inline .with_message()
        if let Some(msg) = custom_message {
            return interpolate_message(msg, &all_params);
        }

        // Priority 2: validator-level custom_message
        if let Some(msg) = self
            .custom_messages
            .get(&(field.to_string(), code.to_string()))
        {
            return interpolate_message(msg, &all_params);
        }

        // Priority 3 & 4: i18n lookup
        if let Ok(manager) = self.app.i18n() {
            let locale = self.locale.as_deref().unwrap_or(manager.default_locale());

            // Try validation.custom.{field}.{code}
            let custom_key = format!("validation.custom.{}.{}", field, code);
            let result = manager.translate(locale, &custom_key, &all_params);
            if result != custom_key {
                return result;
            }

            // Try validation.{code}
            let default_key = format!("validation.{}", code);
            let result = manager.translate(locale, &default_key, &all_params);
            if result != default_key {
                return result;
            }
        }

        // Priority 5: hardcoded fallback
        fallback_message(&attribute, code, params)
    }
}
