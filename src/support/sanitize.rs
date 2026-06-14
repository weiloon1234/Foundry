use std::collections::HashSet;

const ALWAYS_DENIED_TAGS: &[&str] = &[
    "applet", "base", "embed", "frame", "frameset", "iframe", "link", "math", "meta", "object",
    "script", "style", "svg", "template",
];

/// Sanitize HTML by whitelisting allowed tags and stripping everything else.
///
/// Uses the `ammonia` crate for robust HTML parsing that handles malformed HTML,
/// nested tag attacks (`<scr<script>ipt>`), and browser parsing quirks.
/// Active document tags such as `script`, `style`, `iframe`, `object`, `svg`,
/// and `math` are always denied even if included in `allowed_tags`.
///
/// ```rust
/// use foundry::support::sanitize_html;
///
/// let safe = sanitize_html(
///     "<p>Hello <b>world</b></p><script>alert(1)</script>",
///     &["p", "b", "i", "em", "strong"],
/// );
/// assert_eq!(safe, "<p>Hello <b>world</b></p>");
/// ```
pub fn sanitize_html(input: &str, allowed_tags: &[&str]) -> String {
    let tags = safe_allowed_tags(allowed_tags);
    ammonia::Builder::default()
        .tags(tags)
        .clean(input)
        .to_string()
}

/// Strip all HTML tags from input, keeping only text content.
pub fn strip_tags(input: &str) -> String {
    ammonia::Builder::default()
        .tags(HashSet::new())
        .clean(input)
        .to_string()
}

fn safe_allowed_tags<'a>(allowed_tags: &'a [&'a str]) -> HashSet<&'a str> {
    allowed_tags
        .iter()
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty() && !is_always_denied_tag(tag))
        .collect()
}

fn is_always_denied_tag(tag: &str) -> bool {
    ALWAYS_DENIED_TAGS
        .iter()
        .any(|denied| tag.eq_ignore_ascii_case(denied))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_script_tags() {
        let input = "<p>Hello</p><script>alert(1)</script><p>World</p>";
        let result = sanitize_html(input, &["p"]);
        assert_eq!(result, "<p>Hello</p><p>World</p>");
    }

    #[test]
    fn keeps_allowed_tags() {
        let input = "<b>bold</b> and <i>italic</i> and <u>underline</u>";
        let result = sanitize_html(input, &["b", "i"]);
        assert_eq!(result, "<b>bold</b> and <i>italic</i> and underline");
    }

    #[test]
    fn strips_all_tags_when_empty_allowlist() {
        let input = "<b>bold</b> text <i>here</i>";
        assert_eq!(strip_tags(input), "bold text here");
    }

    #[test]
    fn strips_event_handler_attributes() {
        let input = r#"<a href="https://example.com" onclick="alert(1)">link</a>"#;
        let result = sanitize_html(input, &["a"]);
        assert!(result.contains("https://example.com"));
        assert!(!result.contains("onclick"));
    }

    #[test]
    fn strips_javascript_uri() {
        let input = r#"<a href="javascript:alert(1)">link</a>"#;
        let result = sanitize_html(input, &["a"]);
        assert!(!result.contains("javascript"));
    }

    #[test]
    fn handles_nested_tag_attack() {
        let input = "<scr<script>ipt>alert(1)</scr</script>ipt>";
        let result = sanitize_html(input, &["p", "b"]);
        assert!(!result.contains("<script"));
        assert!(!result.contains("</script"));
    }

    #[test]
    fn handles_empty_input() {
        assert_eq!(sanitize_html("", &["p"]), "");
        assert_eq!(strip_tags(""), "");
    }

    #[test]
    fn handles_no_html() {
        assert_eq!(sanitize_html("plain text", &["p"]), "plain text");
    }

    #[test]
    fn case_insensitive_tag_matching() {
        let input = "<B>bold</B>";
        let result = sanitize_html(input, &["b"]);
        assert!(result.contains("bold"));
    }

    #[test]
    fn active_tags_are_denied_even_when_allowed() {
        let input = r#"<p>ok</p><script>alert(1)</script><iframe src="https://example.com"></iframe><svg onload="alert(1)"></svg>"#;
        let result = sanitize_html(input, &["p", "script", "iframe", "svg"]);

        assert_eq!(result, "<p>ok</p>");
    }

    #[test]
    fn allowed_tags_are_trimmed_and_empty_entries_ignored() {
        let input = "<B>bold</B><i>italic</i>";
        let result = sanitize_html(input, &[" b ", ""]);

        assert_eq!(result, "<b>bold</b>italic");
    }
}
