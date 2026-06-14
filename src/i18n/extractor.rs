use std::sync::Arc;

use axum::extract::FromRef;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::Response;

use crate::foundation::AppContext;
use crate::i18n::{I18nManager, Locale};

/// Axum extractor providing translation access in handlers.
///
/// Resolves the per-request locale from the `Accept-Language` header (or from
/// a [`Locale`] extension set by custom middleware) and bundles it with the
/// [`I18nManager`] for translation lookups.
///
/// When i18n is not configured (no `[i18n]` section in config), the extractor
/// degrades gracefully — `t()` returns the key as-is.
///
/// ```
/// use foundry::prelude::*;
/// use foundry::t;
///
/// async fn handler(i18n: I18n) -> String {
///     // No parameters
///     t!(i18n, "Something went wrong")
/// }
///
/// async fn greeting(i18n: I18n) -> String {
///     // Named parameters
///     t!(i18n, "Hello, {{name}}", name = "WeiLoon")
/// }
/// ```
#[derive(Clone)]
pub struct I18n {
    locale: String,
    manager: Option<Arc<I18nManager>>,
}

impl I18n {
    /// Translate a key with no interpolation values.
    ///
    /// Fallback chain: requested locale → fallback locale → key itself.
    pub fn t(&self, key: &str) -> String {
        match &self.manager {
            Some(manager) => manager.translate(&self.locale, key, &[]),
            None => key.to_string(),
        }
    }

    /// Translate a key with interpolation values.
    ///
    /// Replaces `{{placeholder}}` patterns in the translated string.
    pub fn t_with(&self, key: &str, values: &[(&str, &str)]) -> String {
        match &self.manager {
            Some(manager) => manager.translate(&self.locale, key, values),
            None => key.to_string(),
        }
    }

    /// The resolved locale for this request.
    pub fn locale(&self) -> &str {
        &self.locale
    }

    /// Construct an I18n instance from raw parts (for testing).
    #[cfg(test)]
    pub fn from_parts_for_test(locale: String, manager: Option<Arc<I18nManager>>) -> Self {
        Self { locale, manager }
    }
}

impl<S> FromRequestParts<S> for I18n
where
    S: Send + Sync,
    AppContext: FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        let app = AppContext::from_ref(state);

        let manager = app.i18n().ok();

        let locale = parts
            .extensions
            .get::<Locale>()
            .map(|l| l.0.clone())
            .unwrap_or_else(|| match &manager {
                Some(m) => parts
                    .headers
                    .get("accept-language")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| m.resolve_locale(s))
                    .unwrap_or_else(|| m.default_locale().to_string()),
                None => "en".to_string(),
            });

        Ok(Self { locale, manager })
    }
}

/// Axum extractor for just the locale string, without the translation manager.
///
/// Useful when you need to know the request locale but don't need to translate.
impl<S> FromRequestParts<S> for Locale
where
    S: Send + Sync,
    AppContext: FromRef<S>,
{
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> std::result::Result<Self, Self::Rejection> {
        let app = AppContext::from_ref(state);

        // Check if locale was already set (by custom middleware)
        if let Some(locale) = parts.extensions.get::<Locale>() {
            return Ok(locale.clone());
        }

        // Detect from Accept-Language header
        let locale = match app.i18n() {
            Ok(manager) => parts
                .headers
                .get("accept-language")
                .and_then(|v| v.to_str().ok())
                .map(|s| manager.resolve_locale(s))
                .unwrap_or_else(|| manager.default_locale().to_string()),
            Err(_) => "en".to_string(),
        };

        Ok(Locale(locale))
    }
}
