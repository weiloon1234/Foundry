use std::borrow::Cow;
use std::collections::HashMap;

use crate::foundation::AppContext;
use crate::validation::executor::{fallback_message, interpolate_message};
use crate::validation::extractor::RequestValidator;
use crate::validation::field::{EachValidator, FieldValidator, KeyValidator};
use crate::validation::types::{FieldError, ValidationError, ValidationErrors};

fn base_field_name(field: &str) -> Cow<'_, str> {
    if !field.contains('[') {
        return Cow::Borrowed(field);
    }

    let mut base = String::with_capacity(field.len());
    let mut skipping_index = false;
    for ch in field.chars() {
        match ch {
            '[' => skipping_index = true,
            ']' if skipping_index => skipping_index = false,
            _ if !skipping_index => base.push(ch),
            _ => {}
        }
    }

    Cow::Owned(base.trim_start_matches('.').to_string())
}

fn prefixed_field_name(prefix: &str, field: &str) -> String {
    if field.is_empty() {
        prefix.to_string()
    } else if field.starts_with('[') {
        format!("{prefix}{field}")
    } else {
        format!("{prefix}.{field}")
    }
}

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
        value: impl ToString,
    ) -> FieldValidator<'a> {
        FieldValidator {
            validator: self,
            field: name.into(),
            value: value.to_string(),
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
        T: ToString,
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

    pub fn keys<'a, I, K>(&'a mut self, name: impl Into<String>, keys: I) -> KeyValidator<'a>
    where
        I: IntoIterator<Item = K>,
        K: ToString,
    {
        self.key_set(
            name,
            Some(keys.into_iter().map(|key| key.to_string()).collect()),
        )
    }

    #[doc(hidden)]
    pub fn key_set<'a>(
        &'a mut self,
        name: impl Into<String>,
        keys: Option<Vec<String>>,
    ) -> KeyValidator<'a> {
        KeyValidator {
            validator: self,
            field: name.into(),
            keys,
            required_keys: Vec::new(),
            message: None,
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
        self.add_error_with_message(field, code, params, None);
    }

    /// Add a validation error with an optional inline message override.
    pub fn add_error_with_message(
        &mut self,
        field: &str,
        code: &str,
        params: &[(&str, &str)],
        custom_message: Option<&str>,
    ) {
        let msg = self.resolve_message(field, code, params, custom_message);
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

    pub async fn nested<T>(
        &mut self,
        field: impl AsRef<str>,
        value: &T,
    ) -> crate::foundation::Result<()>
    where
        T: RequestValidator + ?Sized,
    {
        let field = field.as_ref();
        let start = self.errors.len();
        let messages = value.messages();
        let attributes = value.attributes();

        let previous_messages = messages
            .iter()
            .map(|(field, code, message)| {
                let key = (field.clone(), code.clone());
                let previous = self.custom_messages.insert(key.clone(), message.clone());
                (key, previous)
            })
            .collect::<Vec<_>>();
        let previous_attributes = attributes
            .iter()
            .map(|(field, name)| {
                let previous = self.custom_attributes.insert(field.clone(), name.clone());
                (field.clone(), previous)
            })
            .collect::<Vec<_>>();

        let result = value.validate(self).await;

        for (key, previous) in previous_messages {
            if let Some(previous) = previous {
                self.custom_messages.insert(key, previous);
            } else {
                self.custom_messages.remove(&key);
            }
        }
        for (key, previous) in previous_attributes {
            if let Some(previous) = previous {
                self.custom_attributes.insert(key, previous);
            } else {
                self.custom_attributes.remove(&key);
            }
        }

        self.prefix_errors_since(start, field);
        result?;
        Ok(())
    }

    pub async fn each_nested<T>(
        &mut self,
        field: impl AsRef<str>,
        items: &[T],
    ) -> crate::foundation::Result<()>
    where
        T: RequestValidator,
    {
        let field = field.as_ref();
        for (index, item) in items.iter().enumerate() {
            let item_field = format!("{field}[{index}]");
            self.nested(item_field, item).await?;
        }
        Ok(())
    }

    fn prefix_errors_since(&mut self, start: usize, prefix: &str) {
        for error in &mut self.errors[start..] {
            error.field = prefixed_field_name(prefix, &error.field);
        }
    }

    pub(crate) fn resolve_field_attribute(&self, field: &str) -> String {
        let base_field = base_field_name(field);

        // Priority 1: validator-level custom_attribute (exact match)
        if let Some(name) = self.custom_attributes.get(field) {
            return self.resolve_attribute_label(name);
        }
        // Priority 1b: validator-level custom_attribute (base field match)
        if base_field.as_ref() != field {
            if let Some(name) = self.custom_attributes.get(base_field.as_ref()) {
                return self.resolve_attribute_label(name);
            }
        }
        // Priority 2: i18n validation.attributes.{field}
        if let Ok(manager) = self.app.i18n() {
            let locale = self.locale.as_deref().unwrap_or(manager.default_locale());
            let key = format!("validation.attributes.{}", base_field.as_ref());
            let resolved = manager.translate(locale, &key, &[]);
            if resolved != key {
                return resolved;
            }
        }
        // Priority 3: raw field name
        base_field.into_owned()
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
        let base_field = base_field_name(field);
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
        if base_field.as_ref() != field {
            if let Some(msg) = self
                .custom_messages
                .get(&(base_field.to_string(), code.to_string()))
            {
                return interpolate_message(msg, &all_params);
            }
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
            if base_field.as_ref() != field {
                let custom_key = format!("validation.custom.{}.{}", base_field.as_ref(), code);
                let result = manager.translate(locale, &custom_key, &all_params);
                if result != custom_key {
                    return result;
                }
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
