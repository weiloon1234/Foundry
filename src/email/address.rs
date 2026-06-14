use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::foundation::{Error, Result};

/// An email address with an optional display name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EmailAddress {
    address: String,
    name: Option<String>,
}

impl EmailAddress {
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            name: None,
        }
    }

    pub fn with_name(address: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            name: Some(name.into()),
        }
    }

    pub fn address(&self) -> &str {
        &self.address
    }
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl From<&str> for EmailAddress {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for EmailAddress {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl std::fmt::Display for EmailAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format_address(self))
    }
}

pub(crate) fn validate_address(addr: &EmailAddress, field: &str) -> Result<()> {
    if contains_forbidden_email_control(addr.address()) {
        return Err(Error::message(format!(
            "email {field} address contains control characters"
        )));
    }
    lettre::Address::from_str(addr.address()).map_err(|error| {
        Error::message(format!(
            "invalid email {field} address '{}': {error}",
            addr.address()
        ))
    })?;

    if let Some(name) = addr.name() {
        if contains_forbidden_email_control(name) {
            return Err(Error::message(format!(
                "email {field} display name contains control characters"
            )));
        }
    }

    Ok(())
}

pub(crate) fn format_address(addr: &EmailAddress) -> String {
    match addr.name() {
        Some(name) if !name.is_empty() => {
            format!("{} <{}>", format_display_name(name), addr.address())
        }
        _ => addr.address().to_string(),
    }
}

fn format_display_name(name: &str) -> String {
    if !needs_quoted_display_name(name) {
        return name.to_string();
    }

    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn needs_quoted_display_name(name: &str) -> bool {
    name.chars().any(|ch| {
        matches!(
            ch,
            '"' | '\\' | '(' | ')' | '<' | '>' | ',' | ';' | ':' | '@' | '[' | ']'
        )
    })
}

fn contains_forbidden_email_control(value: &str) -> bool {
    value.chars().any(|ch| ch.is_control() && ch != '\t')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn email_address_new_creates_without_name() {
        let email = EmailAddress::new("test@example.com");
        assert_eq!(email.address(), "test@example.com");
        assert_eq!(email.name(), None);
    }

    #[test]
    fn email_address_with_name_sets_both() {
        let email = EmailAddress::with_name("test@example.com", "Test User");
        assert_eq!(email.address(), "test@example.com");
        assert_eq!(email.name(), Some("Test User"));
    }

    #[test]
    fn email_address_display_without_name() {
        let email = EmailAddress::new("test@example.com");
        assert_eq!(email.to_string(), "test@example.com");
    }

    #[test]
    fn email_address_display_with_name() {
        let email = EmailAddress::with_name("test@example.com", "Test User");
        assert_eq!(email.to_string(), "Test User <test@example.com>");
    }

    #[test]
    fn format_address_quotes_special_display_names() {
        let email = EmailAddress::with_name("test@example.com", "User, Example");
        assert_eq!(email.to_string(), "\"User, Example\" <test@example.com>");
    }

    #[test]
    fn validate_address_rejects_invalid_addresses_and_control_names() {
        let invalid = EmailAddress::new("bad\r\nbcc@example.com");
        assert!(validate_address(&invalid, "to").is_err());

        let invalid = EmailAddress::with_name("test@example.com", "Bad\nName");
        assert!(validate_address(&invalid, "from").is_err());

        let valid = EmailAddress::with_name("test@example.com", "Test User");
        assert!(validate_address(&valid, "from").is_ok());
    }

    #[test]
    fn email_address_from_str() {
        let email: EmailAddress = "test@example.com".into();
        assert_eq!(email.address(), "test@example.com");
        assert_eq!(email.name(), None);
    }

    #[test]
    fn email_address_from_string() {
        let email: EmailAddress = "test@example.com".to_string().into();
        assert_eq!(email.address(), "test@example.com");
        assert_eq!(email.name(), None);
    }

    #[test]
    fn email_address_serialization_roundtrip() {
        let original = EmailAddress::with_name("test@example.com", "Test User");
        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: EmailAddress = serde_json::from_str(&serialized).unwrap();
        assert_eq!(original, deserialized);
    }
}
