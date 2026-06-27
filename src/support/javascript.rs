use crate::foundation::{Error, Result};

pub(crate) const JAVASCRIPT_MAX_SAFE_INTEGER: u128 = 9_007_199_254_740_991;

pub(crate) fn ensure_safe_integer_u64(context: &str, value: u64, language: &str) -> Result<()> {
    if u128::from(value) > JAVASCRIPT_MAX_SAFE_INTEGER {
        return Err(Error::message(format!(
            "{context} `{value}` is above {language}'s safe integer limit `{JAVASCRIPT_MAX_SAFE_INTEGER}`"
        )));
    }

    Ok(())
}

pub(crate) fn ensure_safe_integer_usize(context: &str, value: usize, language: &str) -> Result<()> {
    if value as u128 > JAVASCRIPT_MAX_SAFE_INTEGER {
        return Err(Error::message(format!(
            "{context} `{value}` is above {language}'s safe integer limit `{JAVASCRIPT_MAX_SAFE_INTEGER}`"
        )));
    }

    Ok(())
}

pub(crate) fn is_typescript_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if !(first == '_' || first == '$' || first.is_ascii_alphabetic()) {
        return false;
    }

    chars.all(|ch| ch == '_' || ch == '$' || ch.is_ascii_alphanumeric())
}

pub(crate) fn to_camel_case_identifier_with_context(value: &str, context: &str) -> Result<String> {
    let mut words = Vec::new();
    let mut current = String::new();

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch);
        } else if matches!(ch, '_' | '-' | ':' | ' ') {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else {
            return Err(Error::message(format!(
                "{context} only supports ASCII property keys; `{value}` contains unsupported character `{ch}`"
            )));
        }
    }

    if !current.is_empty() {
        words.push(current);
    }

    if words.is_empty() {
        return Err(Error::message(format!(
            "{context} requires non-empty property keys"
        )));
    }

    let mut identifier = words[0].to_ascii_lowercase();
    for word in words.iter().skip(1) {
        let lower = word.to_ascii_lowercase();
        let mut chars = lower.chars();
        if let Some(first) = chars.next() {
            identifier.push(first.to_ascii_uppercase());
            identifier.push_str(chars.as_str());
        }
    }

    if !is_typescript_identifier(&identifier) {
        return Err(Error::message(format!(
            "{context} normalized `{value}` to invalid TypeScript identifier `{identifier}`"
        )));
    }

    Ok(identifier)
}
