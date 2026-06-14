use serde::{Deserialize, Serialize};

/// Unresolved email attachment — serializable for queue delivery.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EmailAttachment {
    /// Read attachment bytes from a filesystem path.
    Path {
        path: String,
        name: Option<String>,
        content_type: Option<String>,
    },
    /// Read attachment bytes from a Foundry storage disk.
    Storage {
        disk: Option<String>,
        path: String,
        name: Option<String>,
        content_type: Option<String>,
    },
}

impl EmailAttachment {
    pub fn from_path(path: impl Into<String>) -> Self {
        Self::Path {
            path: path.into(),
            name: None,
            content_type: None,
        }
    }

    pub fn from_storage(disk: impl Into<String>, path: impl Into<String>) -> Self {
        Self::Storage {
            disk: Some(disk.into()),
            path: path.into(),
            name: None,
            content_type: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        match &mut self {
            Self::Path { name: n, .. } => *n = Some(name.into()),
            Self::Storage { name: n, .. } => *n = Some(name.into()),
        }
        self
    }

    pub fn with_content_type(mut self, ct: impl Into<String>) -> Self {
        match &mut self {
            Self::Path {
                content_type: c, ..
            } => *c = Some(ct.into()),
            Self::Storage {
                content_type: c, ..
            } => *c = Some(ct.into()),
        }
        self
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Path { name, .. } | Self::Storage { name, .. } => name.as_deref(),
        }
    }

    pub fn content_type(&self) -> Option<&str> {
        match self {
            Self::Path { content_type, .. } | Self::Storage { content_type, .. } => {
                content_type.as_deref()
            }
        }
    }

    pub fn path(&self) -> &str {
        match self {
            Self::Path { path, .. } | Self::Storage { path, .. } => path,
        }
    }
}

/// Fully resolved attachment with loaded bytes — not serializable (contains raw bytes).
#[derive(Clone, Debug)]
pub struct ResolvedAttachment {
    pub content: Vec<u8>,
    pub name: String,
    pub content_type: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn from_path_creates_path_variant() {
        let attachment = EmailAttachment::from_path("/path/to/file.txt");
        match attachment {
            EmailAttachment::Path {
                path,
                name,
                content_type,
            } => {
                assert_eq!(path, "/path/to/file.txt");
                assert_eq!(name, None);
                assert_eq!(content_type, None);
            }
            _ => panic!("Expected Path variant"),
        }
    }

    #[test]
    fn from_storage_creates_storage_variant() {
        let attachment = EmailAttachment::from_storage("s3", "/path/to/file.pdf");
        match attachment {
            EmailAttachment::Storage {
                disk,
                path,
                name,
                content_type,
            } => {
                assert_eq!(disk, Some("s3".to_string()));
                assert_eq!(path, "/path/to/file.pdf");
                assert_eq!(name, None);
                assert_eq!(content_type, None);
            }
            _ => panic!("Expected Storage variant"),
        }
    }

    #[test]
    fn with_name_sets_name() {
        let attachment = EmailAttachment::from_path("/path/to/file.txt").with_name("document.txt");
        assert_eq!(attachment.name(), Some("document.txt"));
    }

    #[test]
    fn with_content_type_sets_ct() {
        let attachment =
            EmailAttachment::from_path("/path/to/file.txt").with_content_type("text/plain");
        match attachment {
            EmailAttachment::Path { content_type, .. } => {
                assert_eq!(content_type, Some("text/plain".to_string()));
            }
            _ => panic!("Expected Path variant"),
        }
    }

    #[test]
    fn builder_chain_creates_full_attachment() {
        let attachment = EmailAttachment::from_path("/path/to/image.jpg")
            .with_name("photo.jpg")
            .with_content_type("image/jpeg");

        assert_eq!(attachment.name(), Some("photo.jpg"));
        assert_eq!(attachment.path(), "/path/to/image.jpg");

        match attachment {
            EmailAttachment::Path { content_type, .. } => {
                assert_eq!(content_type, Some("image/jpeg".to_string()));
            }
            _ => panic!("Expected Path variant"),
        }
    }

    #[test]
    fn serialization_roundtrip_path() {
        let original = EmailAttachment::from_path("/path/to/file.txt")
            .with_name("test.txt")
            .with_content_type("text/plain");

        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: EmailAttachment = serde_json::from_str(&serialized).unwrap();

        assert_eq!(original, deserialized);
    }

    #[test]
    fn serialization_roundtrip_storage() {
        let original = EmailAttachment::from_storage("local", "/path/to/document.pdf")
            .with_name("report.pdf")
            .with_content_type("application/pdf");

        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: EmailAttachment = serde_json::from_str(&serialized).unwrap();

        assert_eq!(original, deserialized);
    }
}
