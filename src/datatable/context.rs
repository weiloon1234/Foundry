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
    pub locale: Option<String>,
    pub timezone: Timezone,
}

#[derive(Clone)]
struct ScopedDatatableContext {
    locale: Option<String>,
    timezone: Timezone,
}

tokio::task_local! {
    static SCOPED_DATATABLE_CONTEXT: ScopedDatatableContext;
}

impl<'a> DatatableContext<'a> {
    pub fn new(
        app: &'a AppContext,
        actor: Option<&'a Actor>,
        request: &'a DatatableRequest,
    ) -> Self {
        let scoped = SCOPED_DATATABLE_CONTEXT.try_with(Clone::clone).ok();
        Self {
            app,
            actor,
            request,
            locale: scoped
                .as_ref()
                .and_then(|context| context.locale.clone())
                .or_else(|| Some(crate::translations::current_locale(app))),
            timezone: scoped.map_or_else(
                || app.timezone().unwrap_or_else(|_| Timezone::utc()),
                |context| context.timezone,
            ),
        }
    }

    pub fn with_locale_and_timezone(
        app: &'a AppContext,
        actor: Option<&'a Actor>,
        request: &'a DatatableRequest,
        locale: Option<String>,
        timezone: Timezone,
    ) -> Self {
        Self {
            app,
            actor,
            request,
            locale,
            timezone,
        }
    }

    /// Translate a key using the configured i18n system.
    ///
    /// Falls back to returning the key itself if i18n is not configured
    /// or the locale is not available.
    pub fn t(&self, key: &str) -> String {
        let locale = match self.locale.as_deref() {
            Some(locale) => locale,
            None => return key.to_string(),
        };

        match self.app.i18n() {
            Ok(i18n) => i18n.translate(locale, key, &[]),
            Err(_) => key.to_string(),
        }
    }
}

pub(crate) async fn scope_datatable_context<F>(
    locale: Option<String>,
    timezone: Timezone,
    future: F,
) -> F::Output
where
    F: std::future::Future,
{
    SCOPED_DATATABLE_CONTEXT
        .scope(ScopedDatatableContext { locale, timezone }, future)
        .await
}

#[cfg(test)]
mod tests {
    use super::{scope_datatable_context, DatatableContext};
    use crate::config::ConfigRepository;
    use crate::datatable::DatatableRequest;
    use crate::foundation::{AppContext, Container};
    use crate::support::Timezone;
    use crate::validation::RuleRegistry;

    fn test_app() -> AppContext {
        AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn scoped_locale_and_timezone_override_runtime_defaults() {
        let app = test_app();
        let request = DatatableRequest {
            page: 1,
            per_page: 20,
            sort: Vec::new(),
            filters: Vec::new(),
            search: None,
        };
        let timezone = Timezone::parse("Asia/Kuala_Lumpur").unwrap();

        scope_datatable_context(Some("ms".to_string()), timezone.clone(), async {
            let context = DatatableContext::new(&app, None, &request);
            assert_eq!(context.locale.as_deref(), Some("ms"));
            assert_eq!(context.timezone, timezone);
        })
        .await;
    }
}
