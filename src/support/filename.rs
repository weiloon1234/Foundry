pub(crate) fn sanitize_filename(name: &str, fallback: &str, max_bytes: usize) -> String {
    let segment = name
        .rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(name);
    let cleaned: String = segment
        .chars()
        .filter(|ch| !ch.is_control() && *ch != '/' && *ch != '\\')
        .collect();
    let trimmed = cleaned
        .trim_matches(|ch: char| ch.is_whitespace() || ch == '"' || ch == '\'')
        .trim();

    let name = if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        fallback
    } else {
        trimmed
    };

    truncate_filename(name, fallback, max_bytes)
}

pub(crate) fn safe_extension_from_name(name: &str) -> Option<String> {
    let (_, ext) = name.rsplit_once('.')?;
    if ext.is_empty() || ext.len() > 32 {
        return None;
    }
    if !ext
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

fn truncate_filename(name: &str, fallback: &str, max_bytes: usize) -> String {
    if name.len() <= max_bytes {
        return name.to_string();
    }

    let extension = safe_extension_from_name(name);
    let suffix = extension
        .as_ref()
        .map(|ext| format!(".{ext}"))
        .unwrap_or_default();
    let base_limit = max_bytes.saturating_sub(suffix.len());
    let base = name
        .rsplit_once('.')
        .map(|(base, _)| base)
        .filter(|_| extension.is_some())
        .unwrap_or(name);
    let mut truncated = truncate_to_byte_len(base, base_limit);
    if truncated.is_empty() {
        truncated = fallback.to_string();
    }
    truncated.push_str(&suffix);
    truncated
}

pub(crate) fn truncate_to_byte_len(value: &str, limit: usize) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        if output.len() + ch.len_utf8() > limit {
            break;
        }
        output.push(ch);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_handles_paths_controls_quotes_and_fallback() {
        assert_eq!(sanitize_filename("/etc/passwd", "upload", 255), "passwd");
        assert_eq!(
            sanitize_filename(r#"C:\Users\admin\avatar.JPG"#, "upload", 255),
            "avatar.JPG"
        );
        assert_eq!(
            sanitize_filename(" \" report\u{0000}\u{001f}.pdf \" ", "upload", 255),
            "report.pdf"
        );
        assert_eq!(sanitize_filename("////", "upload", 255), "upload");
        assert_eq!(sanitize_filename("..", "upload", 255), "upload");
        assert_eq!(sanitize_filename("照片.png", "upload", 255), "照片.png");
    }

    #[test]
    fn sanitize_filename_caps_long_names_and_preserves_extension() {
        let input = format!("{}.png", "a".repeat(400));
        let name = sanitize_filename(&input, "upload", 255);

        assert!(name.len() <= 255);
        assert!(name.ends_with(".png"));
    }

    #[test]
    fn safe_extension_accepts_only_safe_short_suffixes() {
        assert_eq!(
            safe_extension_from_name("photo.JPG"),
            Some("jpg".to_string())
        );
        assert_eq!(
            safe_extension_from_name("archive.tar.gz"),
            Some("gz".to_string())
        );
        assert_eq!(safe_extension_from_name("file"), None);
        assert_eq!(safe_extension_from_name("file."), None);
        assert_eq!(safe_extension_from_name("file.bad/ext"), None);
        assert_eq!(
            safe_extension_from_name(&format!("file.{}", "a".repeat(33))),
            None
        );
    }
}
