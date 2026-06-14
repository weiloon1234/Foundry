pub(crate) const REDACTED: &str = "[redacted]";

pub(crate) fn redact_sensitive_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if is_sensitive_key(key) {
                    *value = serde_json::Value::String(REDACTED.to_string());
                } else {
                    redact_sensitive_json(value);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_sensitive_json(value);
            }
        }
        _ => {}
    }
}

pub(crate) fn redact_sensitive_text(input: &str) -> String {
    let with_urls = redact_url_credentials(input);
    let with_bearer = redact_bearer_tokens(&with_urls);
    redact_key_value_secrets(&with_bearer)
}

pub(crate) fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    normalized.contains("authorization")
        || normalized.contains("password")
        || normalized.contains("secret")
        || normalized.contains("signature")
        || normalized.contains("credential")
        || normalized.contains("token")
        || normalized.ends_with("key")
        || normalized.contains("_key")
        || normalized.contains("-key")
}

pub(crate) fn redact_url_credentials(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;

    while let Some(relative_scheme_end) = input[cursor..].find("://") {
        let scheme_end = cursor + relative_scheme_end;
        let authority_start = scheme_end + 3;
        output.push_str(&input[cursor..authority_start]);

        let authority_end = input[authority_start..]
            .find(|ch: char| ch == '/' || ch == '?' || ch == '#' || ch.is_whitespace())
            .map(|offset| authority_start + offset)
            .unwrap_or(input.len());
        let authority = &input[authority_start..authority_end];

        if let Some(at_index) = authority.rfind('@') {
            let userinfo = &authority[..at_index];
            let host = &authority[at_index + 1..];
            if userinfo.contains(':') {
                output.push_str(REDACTED);
                output.push('@');
                output.push_str(host);
            } else {
                output.push_str(authority);
            }
        } else {
            output.push_str(authority);
        }

        cursor = authority_end;
    }

    output.push_str(&input[cursor..]);
    output
}

fn redact_bearer_tokens(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    let lower = input.to_ascii_lowercase();

    while let Some(relative_index) = lower[cursor..].find("bearer") {
        let index = cursor + relative_index;
        let after_word = index + "bearer".len();
        if !is_boundary(input, index)
            || !input[after_word..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
        {
            output.push_str(&input[cursor..after_word]);
            cursor = after_word;
            continue;
        }

        output.push_str(&input[cursor..after_word]);
        let whitespace_end = input[after_word..]
            .find(|ch: char| !ch.is_whitespace())
            .map(|offset| after_word + offset)
            .unwrap_or(input.len());
        output.push_str(&input[after_word..whitespace_end]);

        let token_end = input[whitespace_end..]
            .find(is_value_delimiter)
            .map(|offset| whitespace_end + offset)
            .unwrap_or(input.len());
        if token_end > whitespace_end {
            output.push_str(REDACTED);
        }
        cursor = token_end;
    }

    output.push_str(&input[cursor..]);
    output
}

fn redact_key_value_secrets(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;

    while cursor < input.len() {
        let Some((key_start, key_end)) = next_key(input, cursor) else {
            output.push_str(&input[cursor..]);
            break;
        };

        output.push_str(&input[cursor..key_start]);
        let key = &input[key_start..key_end];
        if !is_sensitive_key(key) {
            output.push_str(key);
            cursor = key_end;
            continue;
        }

        let whitespace_end = skip_whitespace(input, key_end);
        let Some(separator) = input[whitespace_end..].chars().next() else {
            output.push_str(key);
            cursor = key_end;
            continue;
        };
        if !matches!(separator, '=' | ':') {
            output.push_str(key);
            cursor = key_end;
            continue;
        }

        let separator_end = whitespace_end + separator.len_utf8();
        let value_start = skip_whitespace(input, separator_end);
        let Some(value_first) = input[value_start..].chars().next() else {
            output.push_str(&input[key_start..value_start]);
            cursor = value_start;
            continue;
        };
        if key.eq_ignore_ascii_case("authorization") && starts_with_bearer_value(input, value_start)
        {
            output.push_str(&input[key_start..value_start]);
            cursor = value_start;
            continue;
        }

        output.push_str(&input[key_start..value_start]);
        let (redaction_start, value_end, closing_quote) = if matches!(value_first, '"' | '\'') {
            let quoted_start = value_start + value_first.len_utf8();
            let quoted_end = input[quoted_start..]
                .find(value_first)
                .map(|offset| quoted_start + offset)
                .unwrap_or(input.len());
            (quoted_start, quoted_end, Some(value_first))
        } else {
            let value_end = input[value_start..]
                .find(is_value_delimiter)
                .map(|offset| value_start + offset)
                .unwrap_or(input.len());
            (value_start, value_end, None)
        };

        output.push_str(&input[value_start..redaction_start]);
        if value_end > redaction_start {
            output.push_str(REDACTED);
        }
        if let Some(quote) = closing_quote {
            if value_end < input.len() {
                output.push(quote);
                cursor = value_end + quote.len_utf8();
            } else {
                cursor = value_end;
            }
        } else {
            cursor = value_end;
        }
    }

    output
}

fn next_key(input: &str, start: usize) -> Option<(usize, usize)> {
    let mut key_start = None;
    for (offset, ch) in input[start..].char_indices() {
        if is_key_char(ch) {
            key_start = Some(start + offset);
            break;
        }
    }

    let key_start = key_start?;
    let key_end = input[key_start..]
        .find(|ch: char| !is_key_char(ch))
        .map(|offset| key_start + offset)
        .unwrap_or(input.len());

    Some((key_start, key_end))
}

fn skip_whitespace(input: &str, start: usize) -> usize {
    input[start..]
        .find(|ch: char| !ch.is_whitespace())
        .map(|offset| start + offset)
        .unwrap_or(input.len())
}

fn is_key_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')
}

fn is_boundary(input: &str, index: usize) -> bool {
    index == 0
        || input[..index]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_key_char(ch))
}

fn is_value_delimiter(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, ',' | ';' | '&' | ')' | ']' | '}')
}

fn starts_with_bearer_value(input: &str, start: usize) -> bool {
    let candidate = &input[start..];
    if candidate.len() <= "bearer".len() {
        return false;
    }
    candidate[.."bearer".len()].eq_ignore_ascii_case("bearer")
        && candidate["bearer".len()..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_sensitive_json_keys_recursively() {
        let mut value = serde_json::json!({
            "api_key": "secret",
            "nested": { "refreshToken": "abc", "safe": "visible" },
            "items": [{ "password": "pw" }]
        });

        redact_sensitive_json(&mut value);

        assert_eq!(value["api_key"], REDACTED);
        assert_eq!(value["nested"]["refreshToken"], REDACTED);
        assert_eq!(value["nested"]["safe"], "visible");
        assert_eq!(value["items"][0]["password"], REDACTED);
    }

    #[test]
    fn redacts_url_credentials_bearer_tokens_and_key_values() {
        let input = "db=postgres://user:secret@example.test/app authorization: Bearer abc.def token=tok password: \"pw\" safe=value";
        let output = redact_sensitive_text(input);

        assert!(output.contains("postgres://[redacted]@example.test/app"));
        assert!(output.contains("authorization: Bearer [redacted]"));
        assert!(output.contains("token=[redacted]"));
        assert!(output.contains("password: \"[redacted]\""));
        assert!(output.contains("safe=value"));
        assert!(!output.contains("abc.def"));
        assert!(!output.contains("secret@example"));
        assert!(!output.contains("\"pw\""));
    }

    #[test]
    fn redacts_query_string_secrets_without_eating_later_values() {
        let output = redact_sensitive_text("request failed: /callback?token=abc&state=ok");

        assert_eq!(
            output,
            "request failed: /callback?token=[redacted]&state=ok"
        );
    }
}
