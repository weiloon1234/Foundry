use std::ops::{Deref, DerefMut};

use async_trait::async_trait;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRef, FromRequest, Request};
use axum::response::{IntoResponse, Response};
use axum::{http::StatusCode, Json};
use serde::de::DeserializeOwned;

use crate::foundation::{AppContext, Error, Result};
use crate::validation::from_multipart::FromMultipart;
use crate::validation::validator::Validator;

#[async_trait]
pub trait RequestValidator: Send + Sync {
    async fn validate(&self, validator: &mut Validator) -> Result<()>;

    /// Custom validation messages for specific field+rule combinations.
    ///
    /// Key: `(field_name, rule_code)` -> custom message.
    /// Messages support `{{attribute}}` and rule-specific placeholders.
    fn messages(&self) -> Vec<(String, String, String)> {
        Vec::new()
    }

    /// Custom display names for fields.
    ///
    /// Key: `field_name` -> display name (used as `{{attribute}}` in messages).
    fn attributes(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    /// Custom extractor-level request messages for parse/content-type failures.
    ///
    /// These messages are resolved before the DTO exists, so they are static
    /// and keyed the same way as validation messages: `(field, code, message)`.
    ///
    /// The framework uses the synthetic `request` field with codes such as
    /// `invalid_request_body` and `multipart_not_supported`.
    fn request_messages() -> Vec<(String, String, String)>
    where
        Self: Sized,
    {
        Vec::new()
    }

    /// Custom display names for extractor-level request messages.
    fn request_attributes() -> Vec<(String, String)>
    where
        Self: Sized,
    {
        Vec::new()
    }
}

pub struct Validated<T>(pub T);
pub struct JsonValidated<T>(pub T);

impl<T> Deref for Validated<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Validated<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> Deref for JsonValidated<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for JsonValidated<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T, S> FromRequest<S> for Validated<T>
where
    T: DeserializeOwned + RequestValidator + FromMultipart + Send + Sync,
    S: Send + Sync,
    AppContext: FromRef<S>,
{
    type Rejection = Response;

    fn from_request(
        req: Request,
        state: &S,
    ) -> impl std::future::Future<Output = std::result::Result<Self, Self::Rejection>> + Send {
        let app = AppContext::from_ref(state);
        let content_type = req
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        async move {
            let is_multipart = content_type.starts_with("multipart/form-data");
            let locale = resolve_request_locale(&app, req.headers(), req.extensions());

            let value = if is_multipart {
                let mut multipart = axum::extract::Multipart::from_request(req, state)
                    .await
                    .map_err(|rejection| {
                        request_error_response::<T>(
                            &app,
                            locale.as_deref(),
                            rejection.status(),
                            "invalid_request_body",
                        )
                    })?;

                let upload_limits = crate::storage::UploadLimits::from_app(&app);

                crate::storage::scope_upload_limits(upload_limits, async {
                    T::from_multipart(&mut multipart).await
                })
                .await
                .map_err(|error| match error {
                    Error::Http { .. } => error.into_response(),
                    _ => Error::http(StatusCode::BAD_REQUEST.as_u16(), error.to_string())
                        .into_response(),
                })?
            } else {
                let Json(v) = Json::<T>::from_request(req, state).await.map_err(|error| {
                    json_rejection_response::<T>(&app, locale.as_deref(), error)
                })?;
                v
            };

            match validate_value_ref(&value, app, locale).await {
                Ok(()) => Ok(Self(value)),
                Err(response) => {
                    if is_multipart {
                        value.cleanup_multipart_files().await;
                    }
                    Err(response)
                }
            }
        }
    }
}

impl<T, S> FromRequest<S> for JsonValidated<T>
where
    T: DeserializeOwned + RequestValidator + Send + Sync,
    S: Send + Sync,
    AppContext: FromRef<S>,
{
    type Rejection = Response;

    fn from_request(
        req: Request,
        state: &S,
    ) -> impl std::future::Future<Output = std::result::Result<Self, Self::Rejection>> + Send {
        let app = AppContext::from_ref(state);
        let content_type = req
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let locale = resolve_request_locale(&app, req.headers(), req.extensions());

        async move {
            if content_type.starts_with("multipart/form-data") {
                return Err(request_error_response::<T>(
                    &app,
                    locale.as_deref(),
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "multipart_not_supported",
                ));
            }

            let Json(value) = Json::<T>::from_request(req, state)
                .await
                .map_err(|error| json_rejection_response::<T>(&app, locale.as_deref(), error))?;

            validate_value(value, app, locale).await.map(Self)
        }
    }
}

fn json_rejection_response<T>(
    app: &AppContext,
    locale: Option<&str>,
    rejection: JsonRejection,
) -> Response
where
    T: RequestValidator,
{
    request_error_response::<T>(app, locale, rejection.status(), "invalid_request_body")
}

fn request_error_response<T>(
    app: &AppContext,
    locale: Option<&str>,
    status: StatusCode,
    code: &'static str,
) -> Response
where
    T: RequestValidator,
{
    let message = resolve_request_message::<T>(app, locale, code);
    Error::http_with_code(status.as_u16(), message, code).into_response()
}

fn resolve_request_message<T>(app: &AppContext, locale: Option<&str>, code: &str) -> String
where
    T: RequestValidator,
{
    let mut validator = Validator::new(app.clone());
    if let Some(locale) = locale {
        validator.set_locale(locale.to_string());
    }

    for (field, message_code, message) in T::request_messages() {
        validator.custom_message(field, message_code, message);
    }
    for (field, name) in T::request_attributes() {
        validator.custom_attribute(field, name);
    }

    validator.resolve_message("request", code, &[], None)
}

async fn validate_value<T>(
    value: T,
    app: AppContext,
    locale: Option<String>,
) -> std::result::Result<T, Response>
where
    T: RequestValidator + Send + Sync,
{
    validate_value_ref(&value, app, locale).await?;
    Ok(value)
}

async fn validate_value_ref<T>(
    value: &T,
    app: AppContext,
    locale: Option<String>,
) -> std::result::Result<(), Response>
where
    T: RequestValidator + Send + Sync,
{
    let mut validator = Validator::new(app);
    if let Some(locale) = locale {
        validator.set_locale(locale);
    }

    for (field, code, msg) in value.messages() {
        validator.custom_message(field, code, msg);
    }
    for (field, name) in value.attributes() {
        validator.custom_attribute(field, name);
    }

    value
        .validate(&mut validator)
        .await
        .map_err(internal_error)?;
    validator.finish().map_err(IntoResponse::into_response)?;

    Ok(())
}

pub(crate) fn resolve_request_locale(
    app: &AppContext,
    headers: &axum::http::HeaderMap,
    extensions: &axum::http::Extensions,
) -> Option<String> {
    // Check Locale extension first (set by custom middleware)
    if let Some(locale) = extensions.get::<crate::i18n::Locale>() {
        return Some(locale.0.clone());
    }
    // Check Accept-Language header
    if let Ok(manager) = app.i18n() {
        if let Some(header) = headers.get("accept-language").and_then(|v| v.to_str().ok()) {
            if !header.is_empty() {
                return Some(manager.resolve_locale(header));
            }
        }
    }
    None
}

fn internal_error(error: Error) -> Response {
    error.into_response()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use axum::body::Body;
    use axum::extract::FromRequest as _;
    use axum::http::{header, Method, Request};
    use serde::Deserialize;

    use crate::config::ConfigRepository;
    use crate::foundation::{AppContext, Container, Error, Result};
    use crate::storage::{UploadCounters, UploadedFile};
    use crate::validation::RuleRegistry;

    use super::*;

    static LAST_UPLOAD_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

    #[derive(Debug, Deserialize)]
    struct RejectingMultipartUpload {
        upload: UploadedFile,
    }

    #[async_trait]
    impl RequestValidator for RejectingMultipartUpload {
        async fn validate(&self, validator: &mut Validator) -> Result<()> {
            let _ = self.upload.size;
            validator.field("name", "").required().apply().await?;
            Ok(())
        }
    }

    #[async_trait]
    impl FromMultipart for RejectingMultipartUpload {
        async fn from_multipart(
            multipart: &mut axum::extract::Multipart,
        ) -> crate::foundation::Result<Self> {
            let mut upload = None;
            let mut counters = UploadCounters::default();

            while let Some(field) = multipart
                .next_field()
                .await
                .map_err(|error| Error::message(format!("multipart error: {error}")))?
            {
                let field_name = field.name().unwrap_or("").to_string();
                if let Some(file) =
                    UploadedFile::from_multipart_field(field_name, field, &mut counters).await?
                {
                    *LAST_UPLOAD_PATH.lock().unwrap() = Some(file.temp_path.clone());
                    upload = Some(file);
                }
            }

            Ok(Self {
                upload: upload.ok_or_else(|| Error::message("missing upload"))?,
            })
        }

        async fn cleanup_multipart_files(&self) {
            crate::storage::upload::remove_uploaded_temp_file(&self.upload).await;
        }
    }

    fn test_app() -> AppContext {
        AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap()
    }

    fn multipart_upload_request() -> Request<Body> {
        let boundary = "foundry-cleanup-test";
        let body = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"upload\"; filename=\"avatar.txt\"\r\n\
             Content-Type: text/plain\r\n\r\n\
             hello\r\n\
             --{boundary}--\r\n"
        );

        Request::builder()
            .method(Method::POST)
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap()
    }

    #[tokio::test]
    async fn multipart_validation_error_removes_uploaded_temp_file() {
        *LAST_UPLOAD_PATH.lock().unwrap() = None;
        let app = test_app();
        let request = multipart_upload_request();

        let response =
            match Validated::<RejectingMultipartUpload>::from_request(request, &app).await {
                Ok(_) => panic!("multipart validation should fail"),
                Err(response) => response,
            };

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let upload_path = LAST_UPLOAD_PATH
            .lock()
            .unwrap()
            .clone()
            .expect("multipart upload should have been written");
        assert!(!upload_path.exists());
    }
}
