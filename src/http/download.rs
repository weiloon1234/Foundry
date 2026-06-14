use axum::http::HeaderValue;

use crate::support::filename::{sanitize_filename, truncate_to_byte_len};

const FALLBACK_DOWNLOAD_FILENAME: &str = "download";
const MAX_DOWNLOAD_FILENAME_BYTES: usize = 255;
const MAX_ASCII_FALLBACK_BYTES: usize = 180;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentDispositionType {
    Attachment,
    Inline,
}

impl ContentDispositionType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Attachment => "attachment",
            Self::Inline => "inline",
        }
    }
}

pub fn attachment_content_disposition(filename: impl AsRef<str>) -> HeaderValue {
    content_disposition_header(ContentDispositionType::Attachment, filename)
}

pub fn inline_content_disposition(filename: impl AsRef<str>) -> HeaderValue {
    content_disposition_header(ContentDispositionType::Inline, filename)
}

pub fn content_disposition_header(
    disposition: ContentDispositionType,
    filename: impl AsRef<str>,
) -> HeaderValue {
    let value = content_disposition_value(disposition, filename.as_ref());
    HeaderValue::from_str(&value).unwrap_or_else(|_| {
        HeaderValue::from_static("attachment; filename=\"download\"; filename*=UTF-8''download")
    })
}

pub fn content_disposition_value(disposition: ContentDispositionType, filename: &str) -> String {
    let filename = sanitize_filename(
        filename,
        FALLBACK_DOWNLOAD_FILENAME,
        MAX_DOWNLOAD_FILENAME_BYTES,
    );
    let fallback = ascii_filename_fallback(&filename);
    let encoded = rfc5987_encode(&filename);

    format!(
        "{}; filename=\"{}\"; filename*=UTF-8''{}",
        disposition.as_str(),
        fallback,
        encoded
    )
}

fn ascii_filename_fallback(filename: &str) -> String {
    let mut fallback = String::new();
    for ch in filename.chars() {
        if ch.is_ascii_graphic() || ch == ' ' {
            match ch {
                '"' | '\\' | ';' => fallback.push('_'),
                _ => fallback.push(ch),
            }
        } else {
            fallback.push('_');
        }
    }

    let fallback = fallback.trim();
    let fallback = if fallback.is_empty() || fallback == "." || fallback == ".." {
        FALLBACK_DOWNLOAD_FILENAME
    } else {
        fallback
    };

    truncate_to_byte_len(fallback, MAX_ASCII_FALLBACK_BYTES)
}

fn rfc5987_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        if is_rfc5987_attr_char(*byte) {
            encoded.push(*byte as char);
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0f));
        }
    }
    encoded
}

fn is_rfc5987_attr_char(byte: u8) -> bool {
    matches!(
        byte,
        b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'!'
            | b'#'
            | b'$'
            | b'&'
            | b'+'
            | b'-'
            | b'.'
            | b'^'
            | b'_'
            | b'`'
            | b'|'
            | b'~'
    )
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'A' + (value - 10)) as char,
        _ => unreachable!("nibble must be 0..=15"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_value_keeps_safe_ascii_name() {
        assert_eq!(
            content_disposition_value(ContentDispositionType::Attachment, "report.xlsx"),
            "attachment; filename=\"report.xlsx\"; filename*=UTF-8''report.xlsx"
        );
    }

    #[test]
    fn content_disposition_strips_path_like_names() {
        assert_eq!(
            content_disposition_value(ContentDispositionType::Attachment, r#"C:\tmp\report.xlsx"#),
            "attachment; filename=\"report.xlsx\"; filename*=UTF-8''report.xlsx"
        );
        assert_eq!(
            content_disposition_value(ContentDispositionType::Attachment, "/etc/passwd"),
            "attachment; filename=\"passwd\"; filename*=UTF-8''passwd"
        );
    }

    #[test]
    fn content_disposition_prevents_header_injection() {
        let value = content_disposition_value(
            ContentDispositionType::Attachment,
            "evil\r\nSet-Cookie: yes.xlsx",
        );

        assert!(!value.contains('\r'));
        assert!(!value.contains('\n'));
        assert_eq!(
            value,
            "attachment; filename=\"evilSet-Cookie: yes.xlsx\"; filename*=UTF-8''evilSet-Cookie%3A%20yes.xlsx"
        );
    }

    #[test]
    fn content_disposition_escapes_quoted_fallback_and_encodes_unicode() {
        let value =
            content_disposition_value(ContentDispositionType::Attachment, "sales; \"五月\".xlsx");

        assert_eq!(
            value,
            "attachment; filename=\"sales_ ____.xlsx\"; filename*=UTF-8''sales%3B%20%22%E4%BA%94%E6%9C%88%22.xlsx"
        );
    }

    #[test]
    fn content_disposition_supports_inline() {
        assert_eq!(
            content_disposition_value(ContentDispositionType::Inline, "preview.pdf"),
            "inline; filename=\"preview.pdf\"; filename*=UTF-8''preview.pdf"
        );
    }

    #[test]
    fn content_disposition_caps_long_names() {
        let input = format!("{}.xlsx", "a".repeat(400));
        let value = content_disposition_value(ContentDispositionType::Attachment, &input);

        assert!(value.contains("filename=\""));
        assert!(value.contains("filename*=UTF-8''"));
        assert!(value.len() < 600);
        assert!(value.ends_with(".xlsx"));
    }

    #[test]
    fn header_value_is_always_valid() {
        let header = attachment_content_disposition("evil\r\n照片.xlsx");

        assert!(header.to_str().unwrap().starts_with("attachment;"));
    }
}
