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

            validate_value(value, app, locale).await.map(Self)
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

    Ok(value)
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
