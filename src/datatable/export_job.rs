use serde::{Deserialize, Serialize};

use crate::auth::Actor;
use crate::foundation::{AppContext, Result};
use crate::jobs::{Job, JobContext};
use crate::support::{JobId, Timezone};

use super::request::DatatableRequest;
use super::response::{DatatableActorSnapshot, DatatableExportAccepted};

// ---------------------------------------------------------------------------
// Job payload
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct DatatableExportJobPayload {
    pub datatable_id: String,
    pub request: DatatableRequest,
    pub actor: Option<DatatableActorSnapshot>,
    pub locale: Option<String>,
    pub timezone: Timezone,
    pub recipient: String,
}

// ---------------------------------------------------------------------------
// Job implementation
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct DatatableExportJob {
    pub payload: DatatableExportJobPayload,
}

#[async_trait::async_trait]
impl Job for DatatableExportJob {
    const ID: JobId = JobId::new("foundry.datatable_export");

    async fn handle(&self, context: JobContext) -> Result<()> {
        let app = context.app();
        let delivery = resolve_export_delivery(app)?;
        let registry = app.datatables()?;

        let datatable = registry.get(&self.payload.datatable_id).ok_or_else(|| {
            crate::foundation::Error::message(format!(
                "datatable '{}' not found in registry",
                self.payload.datatable_id
            ))
        })?;

        // Reconstruct actor from snapshot
        let actor = self.payload.actor.as_ref().map(Actor::from);

        let request = self.payload.request.clone();

        // Preserve the dispatch-time presentation context across the worker boundary.
        let export = super::context::scope_datatable_context(
            self.payload.locale.clone(),
            self.payload.timezone.clone(),
            datatable.export_file(app, actor.as_ref(), request),
        )
        .await?;

        delivery.deliver_file(export, &self.payload.recipient).await
    }

    fn max_retries(&self) -> Option<u32> {
        Some(3)
    }
}

fn resolve_export_delivery(
    app: &AppContext,
) -> Result<std::sync::Arc<Box<dyn super::export::DatatableExportDelivery>>> {
    app.resolve::<Box<dyn super::export::DatatableExportDelivery>>()
        .map_err(|error| {
            crate::foundation::Error::message(format!(
                "datatable export delivery is not registered: {error}"
            ))
        })
}

// ---------------------------------------------------------------------------
// Dispatch helper
// ---------------------------------------------------------------------------

/// Dispatch a queued datatable export job.
pub async fn dispatch_export<D: super::datatable_trait::Datatable + ?Sized>(
    app: &AppContext,
    actor: Option<&Actor>,
    request: DatatableRequest,
    recipient: &str,
) -> Result<DatatableExportAccepted> {
    let actor_snapshot = actor.map(DatatableActorSnapshot::from);

    let timezone = app.timezone().unwrap_or_else(|_| Timezone::utc());

    let job = DatatableExportJob {
        payload: DatatableExportJobPayload {
            datatable_id: D::ID.to_string(),
            request,
            actor: actor_snapshot,
            locale: Some(crate::translations::current_locale(app)),
            timezone,
            recipient: recipient.to_string(),
        },
    };

    let dispatcher = app.jobs()?;
    dispatcher.dispatch(job).await?;

    Ok(DatatableExportAccepted {
        datatable_id: D::ID.to_string(),
        recipient: recipient.to_string(),
        status: "queued".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::resolve_export_delivery;
    use crate::config::ConfigRepository;
    use crate::foundation::{AppContext, Container};
    use crate::validation::RuleRegistry;

    #[test]
    fn missing_delivery_is_an_explicit_error() {
        let app = AppContext::new(
            Container::new(),
            ConfigRepository::empty(),
            RuleRegistry::new(),
        )
        .unwrap();

        let error = resolve_export_delivery(&app).err().unwrap();
        assert!(error
            .to_string()
            .contains("datatable export delivery is not registered"));
    }
}
