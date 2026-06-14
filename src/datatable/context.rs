use crate::auth::Actor;
use crate::foundation::AppContext;
use crate::support::Timezone;

use super::request::DatatableRequest;

/// Scoped execution context for datatable operations.
///
/// Datatables do **not** rely on `app.current_actor()`. Authorization and scope
/// decisions read from `ctx.actor`, keeping behavior deterministic across
/// HTTP JSON requests, direct export endpoints, and queued export jobs.
pub struct DatatableContext<'a> {
    pub app: &'a AppContext,
    pub actor: Option<&'a Actor>,
    pub request: &'a DatatableRequest,
    pub locale: Option<&'a str>,
    pub timezone: Timezone,
}

impl<'a> DatatableContext<'a> {
    pub fn new(
        app: &'a AppContext,
        actor: Option<&'a Actor>,
        request: &'a DatatableRequest,
    ) -> Self {
        Self {
            app,
            actor,
            request,
            locale: None,
            timezone: app
                .timezone()
                .unwrap_or_else(|_| crate::support::Timezone::utc()),
        }
    }

    /// Translate a key using the configured i18n system.
    ///
    /// Falls back to returning the key itself if i18n is not configured
    /// or the locale is not available.
    pub fn t(&self, key: &str) -> String {
        let locale = match self.locale {
            Some(l) => l,
            None => return key.to_string(),
        };

        match self.app.i18n() {
            Ok(i18n) => i18n.translate(locale, key, &[]),
            Err(_) => key.to_string(),
        }
    }
}
