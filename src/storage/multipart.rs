use std::collections::HashMap;

use axum::extract::FromRef;
use axum::extract::FromRequest;
use axum::response::{IntoResponse, Response};

use crate::foundation::{AppContext, Error, Result};

use super::upload::{
    invalid_multipart_response, remove_uploaded_temp_file, uploaded_file_from_multipart_field,
    UploadCounters, UploadLimits,
};

pub type UploadedFile = super::upload::UploadedFile;

/// Extractor that parses all fields from a multipart/form-data request.
///
/// File fields are collected into [`UploadedFile`] instances grouped by field
/// name; text fields are collected as plain strings.
///
/// # Handler usage
///
/// ```ignore
/// use foundry::storage::{MultipartForm, UploadedFile};
///
/// async fn upload(form: MultipartForm) -> impl IntoResponse {
///     let avatar: &UploadedFile = form.file("avatar")?;
///     let display_name = form.text("name");
///     // ...
/// }
/// ```
#[derive(Debug)]
pub struct MultipartForm {
    files: HashMap<String, Vec<UploadedFile>>,
    texts: HashMap<String, String>,
}

impl MultipartForm {
    /// Returns the first file uploaded under the given field name.
    ///
    /// Returns an error if no file was uploaded for that field.
    pub fn file(&self, name: &str) -> Result<&UploadedFile> {
        self.files
            .get(name)
            .and_then(|v| v.first())
            .ok_or_else(|| Error::message(format!("no file uploaded for field `{name}`")))
    }

    /// Returns all files uploaded under the given field name.
    pub fn files(&self, name: &str) -> &[UploadedFile] {
        self.files.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Returns the text value of a non-file field, or `None` if absent.
    pub fn text(&self, name: &str) -> Option<&str> {
        self.texts.get(name).map(|s| s.as_str())
    }
}

impl<S> FromRequest<S> for MultipartForm
where
    S: Send + Sync,
    AppContext: FromRef<S>,
{
    type Rejection = Response;

    async fn from_request(
        req: axum::http::Request<axum::body::Body>,
        state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        let app = AppContext::from_ref(state);
        let limits = UploadLimits::from_app(&app);

        let mut multipart = axum::extract::Multipart::from_request(req, state)
            .await
            .map_err(|rejection| {
                invalid_multipart_response(rejection.status().as_u16(), rejection)
            })?;

        let mut files: HashMap<String, Vec<UploadedFile>> = HashMap::new();
        let mut texts: HashMap<String, String> = HashMap::new();
        let mut counters = UploadCounters::default();

        while let Some(field) = match multipart.next_field().await {
            Ok(field) => field,
            Err(error) => {
                cleanup_form_files(&files).await;
                return Err(invalid_multipart_response(400, error));
            }
        } {
            let field_name = field.name().unwrap_or("").to_string();

            if field.file_name().is_some() {
                let file = match uploaded_file_from_multipart_field(
                    field_name.clone(),
                    field,
                    limits,
                    &mut counters,
                )
                .await
                {
                    Ok(Some(file)) => file,
                    Ok(None) => continue,
                    Err(error) => {
                        cleanup_form_files(&files).await;
                        return Err(error.into_response());
                    }
                };
                files.entry(field_name).or_default().push(file);
            } else {
                // Text field — collect the full value.
                let text = match field.text().await {
                    Ok(text) => text,
                    Err(error) => {
                        cleanup_form_files(&files).await;
                        return Err(invalid_multipart_response(400, error));
                    }
                };
                texts.insert(field_name, text);
            }
        }

        Ok(MultipartForm { files, texts })
    }
}

async fn cleanup_form_files(files: &HashMap<String, Vec<UploadedFile>>) {
    for file in files.values().flat_map(|items| items.iter()) {
        let _ = remove_uploaded_temp_file(file).await;
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use axum::routing::post;
    use serde_json::Value;
    use tower::ServiceExt as _;

    use crate::config::ConfigRepository;
    use crate::foundation::{AppContext, Container};
    use crate::validation::RuleRegistry;

    use super::*;

    async fn count_files(form: MultipartForm) -> String {
        form.files("file").len().to_string()
    }

    fn app_with_storage_config(config: &str) -> AppContext {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("foundry.toml"), config).unwrap();
        let config = ConfigRepository::from_dir(directory.path()).unwrap();
        AppContext::new(Container::new(), config, RuleRegistry::new()).unwrap()
    }

    fn multipart_request(body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/")
            .header(
                header::CONTENT_TYPE,
                "multipart/form-data; boundary=foundry-test",
            )
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[test]
    fn multipart_form_file_returns_first_file() {
        let mut form = MultipartForm {
            files: HashMap::new(),
            texts: HashMap::new(),
        };

        let uploaded = UploadedFile {
            field_name: "avatar".to_string(),
            original_name: Some("photo.png".to_string()),
            content_type: Some("image/png".to_string()),
            size: 2048,
            temp_path: PathBuf::from("/tmp/test-upload"),
        };

        form.files
            .entry("avatar".to_string())
            .or_default()
            .push(uploaded);

        assert!(form.file("avatar").is_ok());
        assert_eq!(
            form.file("avatar").unwrap().original_name.as_deref(),
            Some("photo.png")
        );
        assert!(form.file("missing").is_err());
    }

    #[test]
    fn multipart_form_files_returns_slice() {
        let mut form = MultipartForm {
            files: HashMap::new(),
            texts: HashMap::new(),
        };

        let f1 = UploadedFile {
            field_name: String::new(),
            original_name: Some("a.txt".to_string()),
            content_type: None,
            size: 10,
            temp_path: PathBuf::from("/tmp/a"),
        };
        let f2 = UploadedFile {
            field_name: String::new(),
            original_name: Some("b.txt".to_string()),
            content_type: None,
            size: 20,
            temp_path: PathBuf::from("/tmp/b"),
        };

        form.files
            .entry("docs".to_string())
            .or_default()
            .extend([f1, f2]);

        assert_eq!(form.files("docs").len(), 2);
        assert!(form.files("missing").is_empty());
    }

    #[test]
    fn multipart_form_text_returns_value() {
        let mut form = MultipartForm {
            files: HashMap::new(),
            texts: HashMap::new(),
        };
        form.texts.insert("name".to_string(), "Foundry".to_string());

        assert_eq!(form.text("name"), Some("Foundry"));
        assert_eq!(form.text("missing"), None);
    }

    #[tokio::test]
    async fn multipart_form_returns_json_error_when_file_limit_is_exceeded() {
        let app = app_with_storage_config(
            r#"
            [storage]
            max_upload_files = 1
            "#,
        );
        let router = axum::Router::new()
            .route("/", post(count_files))
            .with_state(app);
        let body = concat!(
            "--foundry-test\r\n",
            "Content-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "a\r\n",
            "--foundry-test\r\n",
            "Content-Disposition: form-data; name=\"file\"; filename=\"b.txt\"\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "b\r\n",
            "--foundry-test--\r\n"
        );

        let response = router.oneshot(multipart_request(body)).await.unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], "Too many uploaded files");
        assert_eq!(json["error_code"], "too_many_uploaded_files");
    }

    #[tokio::test]
    async fn multipart_form_returns_json_error_when_file_size_is_exceeded() {
        let app = app_with_storage_config(
            r#"
            [storage]
            max_upload_file_size_bytes = 3
            "#,
        );
        let router = axum::Router::new()
            .route("/", post(count_files))
            .with_state(app);
        let body = concat!(
            "--foundry-test\r\n",
            "Content-Disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "abcdef\r\n",
            "--foundry-test--\r\n"
        );

        let response = router.oneshot(multipart_request(body)).await.unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["message"], "Uploaded file is too large");
        assert_eq!(json["error_code"], "uploaded_file_too_large");
    }

    #[tokio::test]
    async fn cleanup_form_files_removes_only_foundry_upload_temps() {
        let dir = tempfile::tempdir().unwrap();
        let foundry_dir = crate::storage::upload::foundry_upload_temp_dir();
        std::fs::create_dir_all(&foundry_dir).unwrap();
        let foundry_temp = foundry_dir.join(format!("foundry-upload-{}", uuid::Uuid::now_v7()));
        let other_temp = dir.path().join("other-upload");
        std::fs::write(&foundry_temp, b"temp").unwrap();
        std::fs::write(&other_temp, b"keep").unwrap();

        let mut files = HashMap::new();
        files.insert(
            "file".to_string(),
            vec![
                UploadedFile {
                    field_name: "file".to_string(),
                    original_name: Some("a.txt".to_string()),
                    content_type: Some("text/plain".to_string()),
                    size: 4,
                    temp_path: foundry_temp.clone(),
                },
                UploadedFile {
                    field_name: "file".to_string(),
                    original_name: Some("b.txt".to_string()),
                    content_type: Some("text/plain".to_string()),
                    size: 4,
                    temp_path: other_temp.clone(),
                },
            ],
        );

        cleanup_form_files(&files).await;

        assert!(!foundry_temp.exists());
        assert!(other_temp.exists());
    }
}
