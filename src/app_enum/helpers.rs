fn push_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
}

fn is_separator(ch: char) -> bool {
    matches!(ch, '_' | '-' | ' ')
}

fn should_split(prev: char, current: char, next: Option<char>) -> bool {
    if is_separator(prev) || is_separator(current) {
        return false;
    }

    (prev.is_lowercase() && current.is_uppercase())
        || (prev.is_alphabetic() && current.is_ascii_digit())
        || (prev.is_ascii_digit() && current.is_alphabetic())
        || (prev.is_uppercase()
            && current.is_uppercase()
            && next.is_some_and(|ch| ch.is_lowercase()))
}

fn split_identifier_words(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();

    for (index, ch) in chars.iter().copied().enumerate() {
        if is_separator(ch) {
            push_token(&mut tokens, &mut current);
            continue;
        }

        if let Some(prev) = current.chars().last() {
            let next = chars.get(index + 1).copied();
            if should_split(prev, ch, next) {
                push_token(&mut tokens, &mut current);
            }
        }

        current.push(ch);
    }

    push_token(&mut tokens, &mut current);
    tokens
}

/// Convert CamelCase to snake_case.
/// Pending -> pending, ActiveIncome -> active_income, HTTP2Enabled -> http_2_enabled
pub fn to_snake_case(name: &str) -> String {
    split_identifier_words(name)
        .into_iter()
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("_")
}

/// Convert CamelCase to title text (insert spaces on identifier boundaries).
/// Pending -> Pending, ActiveIncome -> Active Income, Credit1 -> Credit 1
pub fn to_title_text(name: &str) -> String {
    split_identifier_words(name).join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_simple() {
        assert_eq!(to_snake_case("Pending"), "pending");
    }

    #[test]
    fn snake_case_multi_word() {
        assert_eq!(to_snake_case("ActiveIncome"), "active_income");
    }

    #[test]
    fn snake_case_single_char() {
        assert_eq!(to_snake_case("X"), "x");
    }

    #[test]
    fn snake_case_already_lower() {
        assert_eq!(to_snake_case("active"), "active");
    }

    #[test]
    fn snake_case_splits_digits() {
        assert_eq!(to_snake_case("Credit1"), "credit_1");
    }

    #[test]
    fn snake_case_splits_acronyms_and_digits() {
        assert_eq!(to_snake_case("HTTP2Enabled"), "http_2_enabled");
    }

    #[test]
    fn title_text_simple() {
        assert_eq!(to_title_text("Pending"), "Pending");
    }

    #[test]
    fn title_text_multi_word() {
        assert_eq!(to_title_text("ActiveIncome"), "Active Income");
    }

    #[test]
    fn title_text_single_char() {
        assert_eq!(to_title_text("X"), "X");
    }

    #[test]
    fn title_text_splits_digits() {
        assert_eq!(to_title_text("Credit1"), "Credit 1");
    }

    #[test]
    fn title_text_splits_acronyms_and_digits() {
        assert_eq!(to_title_text("HTTP2Enabled"), "HTTP 2 Enabled");
    }
}
