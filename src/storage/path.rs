use crate::foundation::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StoragePathKind {
    Object,
    Prefix,
}

pub(crate) fn normalize_path(path: &str) -> Result<String> {
    normalize_storage_path(path, StoragePathKind::Object)
}

pub(crate) fn normalize_prefix(prefix: &str) -> Result<String> {
    normalize_storage_path(prefix, StoragePathKind::Prefix)
}

fn normalize_storage_path(value: &str, kind: StoragePathKind) -> Result<String> {
    let label = match kind {
        StoragePathKind::Object => "path",
        StoragePathKind::Prefix => "prefix",
    };

    if value.is_empty() {
        return Err(invalid_storage_path(label, value, "cannot be empty"));
    }
    if value.chars().any(char::is_control) {
        return Err(invalid_storage_path(
            label,
            value,
            "cannot contain control characters",
        ));
    }
    if value.contains('\\') {
        return Err(invalid_storage_path(
            label,
            value,
            "must use forward slashes",
        ));
    }
    if value.starts_with('/') {
        return Err(invalid_storage_path(label, value, "cannot be absolute"));
    }
    if has_windows_drive_prefix(value) {
        return Err(invalid_storage_path(
            label,
            value,
            "cannot use a drive prefix",
        ));
    }

    let mut segments: Vec<&str> = value.split('/').collect();
    if kind == StoragePathKind::Prefix && value.ends_with('/') {
        segments.pop();
    }

    if segments.is_empty() {
        return Err(invalid_storage_path(label, value, "must include a segment"));
    }

    for segment in segments {
        if segment.is_empty() {
            return Err(invalid_storage_path(
                label,
                value,
                "cannot contain empty segments",
            ));
        }
        if segment == "." || segment == ".." {
            return Err(invalid_storage_path(
                label,
                value,
                "cannot contain relative segments",
            ));
        }
    }

    Ok(value.to_string())
}

fn has_windows_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

fn invalid_storage_path(label: &str, value: &str, reason: &str) -> Error {
    Error::message(format!(
        "invalid storage {label} `{}`: {reason}",
        value.escape_debug()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_safe_paths_and_prefixes() {
        assert_eq!(
            normalize_path("attachments/users/avatar.jpg").unwrap(),
            "attachments/users/avatar.jpg"
        );
        assert_eq!(normalize_prefix("attachments/").unwrap(), "attachments/");
        assert_eq!(normalize_prefix("attachments").unwrap(), "attachments");
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        for value in [
            "",
            "/etc/passwd",
            "../secret",
            "attachments/../secret",
            "attachments/./secret",
            "attachments//secret",
            "attachments\\secret",
            "C:/secret",
            "attachments/\nsecret",
        ] {
            assert!(normalize_path(value).is_err(), "{value:?}");
        }
    }

    #[test]
    fn prefix_allows_only_trailing_empty_segment() {
        assert!(normalize_prefix("attachments/").is_ok());
        assert!(normalize_prefix("attachments//").is_err());
        assert!(normalize_prefix("/").is_err());
    }
}
