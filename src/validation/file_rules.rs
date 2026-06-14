use tokio::io::AsyncReadExt as _;

use crate::foundation::{Error, Result};
use crate::storage::UploadedFile;

const MIME_SNIFF_BYTES: usize = 8192;

/// Check if a file is an image by reading magic bytes.
pub async fn is_image(file: &UploadedFile) -> Result<bool> {
    let sample = read_file_sample(file, MIME_SNIFF_BYTES).await?;
    Ok(infer::is_image(&sample))
}

/// Check if file size is within limit (in KB).
pub fn check_max_size(file: &UploadedFile, max_kb: u64) -> bool {
    file.size <= max_kb * 1024
}

/// Check if image dimensions are within limits.
/// Returns (width, height) if the file is a valid image.
pub async fn get_image_dimensions(file: &UploadedFile) -> Result<(u32, u32)> {
    // Stream from disk on the blocking pool instead of slurping the whole
    // upload into memory: dimension extraction only needs the image header,
    // and a large upload would otherwise allocate its full size per check.
    let path = file.temp_path.clone();
    crate::support::run_blocking("validation.image_dimensions", move || {
        let handle = std::fs::File::open(&path)
            .map_err(|e| Error::message(format!("failed to read uploaded file: {e}")))?;
        let reader = image::ImageReader::new(std::io::BufReader::new(handle))
            .with_guessed_format()
            .map_err(|e| Error::message(format!("failed to detect image format: {e}")))?;
        reader
            .into_dimensions()
            .map_err(|e| Error::message(format!("failed to read image dimensions: {e}")))
    })
    .await
}

/// Check if MIME type is in allowed list.
/// Checks magic bytes first, then falls back only for safe text-like MIME types.
pub async fn check_allowed_mimes(file: &UploadedFile, allowed: &[String]) -> Result<bool> {
    let sample = read_file_sample(file, MIME_SNIFF_BYTES).await?;
    if let Some(kind) = infer::get(&sample) {
        return Ok(mime_allowed(kind.mime_type(), allowed));
    }

    if let Some(ref content_type) = file.content_type {
        let content_type = content_type.trim().to_ascii_lowercase();
        let base = content_type.split(';').next().unwrap_or("").trim();
        if is_safe_text_mime(base) && looks_like_text(&sample) {
            return Ok(mime_allowed(base, allowed));
        }
    }

    Ok(false)
}

/// Check if file extension is in allowed list.
pub fn check_allowed_extensions(file: &UploadedFile, allowed: &[String]) -> bool {
    file.original_extension().is_some_and(|ext| {
        allowed
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&ext))
    })
}

async fn read_file_sample(file: &UploadedFile, max_bytes: usize) -> Result<Vec<u8>> {
    let mut handle = tokio::fs::File::open(&file.temp_path)
        .await
        .map_err(|e| Error::message(format!("failed to read uploaded file: {e}")))?;
    let mut sample = vec![0; max_bytes];
    let read = handle
        .read(&mut sample)
        .await
        .map_err(|e| Error::message(format!("failed to read uploaded file: {e}")))?;
    sample.truncate(read);
    Ok(sample)
}

fn mime_allowed(mime: &str, allowed: &[String]) -> bool {
    allowed
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(mime))
}

fn is_safe_text_mime(mime: &str) -> bool {
    matches!(
        mime,
        "text/plain"
            | "text/csv"
            | "text/tab-separated-values"
            | "application/json"
            | "application/xml"
            | "application/csv"
            | "application/x-ndjson"
    )
}

fn looks_like_text(sample: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(sample) else {
        return false;
    };

    text.chars()
        .all(|ch| !ch.is_control() || matches!(ch, '\n' | '\r' | '\t'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_file_with_size(size: u64) -> UploadedFile {
        let temp_dir = std::env::temp_dir().join("foundry-test-file-rules");
        std::fs::create_dir_all(&temp_dir).unwrap();
        let temp_path = temp_dir.join(format!("test-{}", uuid::Uuid::now_v7()));
        std::fs::write(&temp_path, vec![0u8; size as usize]).unwrap();

        UploadedFile {
            field_name: "file".to_string(),
            original_name: Some("test.png".to_string()),
            content_type: Some("image/png".to_string()),
            size,
            temp_path,
        }
    }

    fn make_file_with_content(content: &[u8], name: &str) -> UploadedFile {
        make_file_with_content_type(content, name, Some("application/octet-stream"))
    }

    fn make_file_with_content_type(
        content: &[u8],
        name: &str,
        content_type: Option<&str>,
    ) -> UploadedFile {
        let temp_dir = std::env::temp_dir().join("foundry-test-file-rules");
        std::fs::create_dir_all(&temp_dir).unwrap();
        let temp_path = temp_dir.join(format!("test-{}", uuid::Uuid::now_v7()));
        std::fs::write(&temp_path, content).unwrap();

        UploadedFile {
            field_name: "file".to_string(),
            original_name: Some(name.to_string()),
            content_type: content_type.map(str::to_string),
            size: content.len() as u64,
            temp_path,
        }
    }

    #[test]
    fn check_max_size_within_limit() {
        let file = make_file_with_size(1024 * 100); // 100KB
        assert!(check_max_size(&file, 200)); // 200KB limit
    }

    #[test]
    fn check_max_size_over_limit() {
        let file = make_file_with_size(1024 * 300); // 300KB
        assert!(!check_max_size(&file, 200)); // 200KB limit
    }

    #[test]
    fn check_max_size_exact_limit() {
        let file = make_file_with_size(1024 * 200); // exactly 200KB
        assert!(check_max_size(&file, 200));
    }

    #[test]
    fn check_allowed_extensions_match() {
        let file = make_file_with_size(100);
        let allowed: Vec<String> = vec!["jpg".into(), "png".into(), "webp".into()];
        assert!(check_allowed_extensions(&file, &allowed));
    }

    #[test]
    fn check_allowed_extensions_no_match() {
        let mut file = make_file_with_size(100);
        file.original_name = Some("document.pdf".to_string());
        let allowed: Vec<String> = vec!["jpg".into(), "png".into()];
        assert!(!check_allowed_extensions(&file, &allowed));
    }

    #[test]
    fn check_allowed_extensions_case_insensitive() {
        let mut file = make_file_with_size(100);
        file.original_name = Some("photo.JPG".to_string());
        let allowed: Vec<String> = vec!["jpg".into()];
        assert!(check_allowed_extensions(&file, &allowed));
    }

    #[test]
    fn check_allowed_extensions_uses_sanitized_name() {
        let mut file = make_file_with_size(100);
        file.original_name = Some(r#"C:\fake\photo.PNG"#.to_string());
        let allowed: Vec<String> = vec!["png".into()];
        assert!(check_allowed_extensions(&file, &allowed));
    }

    #[tokio::test]
    async fn is_image_detects_png() {
        // PNG magic bytes
        let png_header = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
        let file = make_file_with_content(&png_header, "test.png");
        assert!(is_image(&file).await.unwrap());
    }

    #[tokio::test]
    async fn is_image_rejects_non_image() {
        let file = make_file_with_content(b"hello world", "test.txt");
        assert!(!is_image(&file).await.unwrap());
    }

    #[tokio::test]
    async fn check_allowed_mimes_accepts_magic_byte_binary_types() {
        let png_header = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
        let png = make_file_with_content_type(&png_header, "photo.png", Some("text/plain"));
        let allowed_png: Vec<String> = vec!["image/png".into()];
        assert!(check_allowed_mimes(&png, &allowed_png).await.unwrap());

        let pdf = make_file_with_content_type(b"%PDF-1.7\n%foundry\n", "doc.pdf", None);
        let allowed_pdf: Vec<String> = vec!["application/pdf".into()];
        assert!(check_allowed_mimes(&pdf, &allowed_pdf).await.unwrap());
    }

    #[tokio::test]
    async fn check_allowed_mimes_rejects_spoofed_binary_content_type() {
        let file = make_file_with_content_type(b"hello world", "fake.png", Some("image/png"));
        let allowed: Vec<String> = vec!["image/png".into()];

        assert!(!check_allowed_mimes(&file, &allowed).await.unwrap());
    }

    #[tokio::test]
    async fn check_allowed_mimes_keeps_text_plain_compatibility_fallback() {
        let file = make_file_with_content_type(b"hello world\n", "notes.txt", Some("text/plain"));
        let allowed: Vec<String> = vec!["text/plain".into()];

        assert!(check_allowed_mimes(&file, &allowed).await.unwrap());
    }

    #[tokio::test]
    async fn check_allowed_mimes_rejects_binary_bytes_pretending_to_be_text() {
        let file =
            make_file_with_content_type(b"hello\0world\xff", "notes.txt", Some("text/plain"));
        let allowed: Vec<String> = vec!["text/plain".into()];

        assert!(!check_allowed_mimes(&file, &allowed).await.unwrap());
    }
}
