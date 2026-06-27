use serde::{Deserialize, Serialize};

use crate::auth::Actor;
use crate::foundation::{AppContext, Result};
use crate::jobs::{Job, JobContext};
use crate::support::{JobId, Timezone};

use super::export::DatatableExportDelivery;
use super::request::DatatableRequest;
use super::response::{DatatableActorSnapshot, DatatableExportAccepted, DatatableExportStatus};

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

        // Use the download path to generate the export data, then deliver
        let response = datatable.download(app, actor.as_ref(), request).await?;

        // Extract bytes from the response body
        let (_, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX).await.map_err(|e| {
            crate::foundation::Error::message(format!("failed to read export body: {e}"))
        })?;

        let export = super::export::GeneratedDatatableExport {
            datatable_id: self.payload.datatable_id.clone(),
            filename: format!("{}.xlsx", self.payload.datatable_id),
            data: bytes.to_vec(),
            columns: Vec::new(),
        };

        // Resolve delivery implementation from container, fall back to no-op
        match app.resolve::<Box<dyn super::export::DatatableExportDelivery>>() {
            Ok(delivery) => delivery.deliver(export, &self.payload.recipient).await,
            Err(_) => {
                super::export::NoopExportDelivery
                    .deliver(export, &self.payload.recipient)
                    .await
            }
        }
    }

    fn max_retries(&self) -> Option<u32> {
        Some(3)
    }
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
            locale: None,
            timezone,
            recipient: recipient.to_string(),
        },
    };

    let dispatcher = app.jobs()?;
    dispatcher.dispatch(job).await?;

    Ok(DatatableExportAccepted {
        datatable_id: D::ID.to_string(),
        recipient: recipient.to_string(),
        status: DatatableExportStatus::Queued,
    })
}
