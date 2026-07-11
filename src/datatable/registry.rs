use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::auth::Actor;
use crate::foundation::{AppContext, Error, Result};
use crate::support::sync::lock_unpoisoned;

use super::datatable_trait::Datatable;
use super::request::DatatableRequest;
use super::response::{DatatableExportAccepted, DatatableJsonResponse};

// ---------------------------------------------------------------------------
// Type-erased datatable interface
// ---------------------------------------------------------------------------

#[async_trait]
pub trait DynDatatable: Send + Sync {
    fn id(&self) -> &str;

    async fn json(
        &self,
        app: &AppContext,
        actor: Option<&Actor>,
        request: DatatableRequest,
    ) -> Result<DatatableJsonResponse>;

    async fn download(
        &self,
        app: &AppContext,
        actor: Option<&Actor>,
        request: DatatableRequest,
    ) -> Result<axum::response::Response>;

    async fn export_file(
        &self,
        app: &AppContext,
        actor: Option<&Actor>,
        request: DatatableRequest,
    ) -> Result<super::export::GeneratedDatatableExportFile> {
        let response = self.download(app, actor, request).await?;
        super::download::response_to_xlsx_file(self.id(), response).await
    }

    async fn queue_email(
        &self,
        app: &AppContext,
        actor: Option<&Actor>,
        request: DatatableRequest,
        recipient: &str,
    ) -> Result<DatatableExportAccepted>;
}

// ---------------------------------------------------------------------------
// Adapter: Datatable -> DynDatatable
// ---------------------------------------------------------------------------

pub struct DatatableAdapter<D>(std::marker::PhantomData<D>);

impl<D> Default for DatatableAdapter<D> {
    fn default() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<D> DatatableAdapter<D> {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl<D> DynDatatable for DatatableAdapter<D>
where
    D: Datatable,
{
    fn id(&self) -> &str {
        D::ID
    }

    async fn json(
        &self,
        app: &AppContext,
        actor: Option<&Actor>,
        request: DatatableRequest,
    ) -> Result<DatatableJsonResponse> {
        D::json(app, actor, request).await
    }

    async fn download(
        &self,
        app: &AppContext,
        actor: Option<&Actor>,
        request: DatatableRequest,
    ) -> Result<axum::response::Response> {
        D::download(app, actor, request).await
    }

    async fn export_file(
        &self,
        app: &AppContext,
        actor: Option<&Actor>,
        request: DatatableRequest,
    ) -> Result<super::export::GeneratedDatatableExportFile> {
        super::download::build_xlsx_file::<D>(app, actor, request).await
    }

    async fn queue_email(
        &self,
        app: &AppContext,
        actor: Option<&Actor>,
        request: DatatableRequest,
        recipient: &str,
    ) -> Result<DatatableExportAccepted> {
        D::queue_email(app, actor, request, recipient).await
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct DatatableRegistry {
    datatables: HashMap<String, Arc<dyn DynDatatable>>,
}

impl DatatableRegistry {
    pub fn get(&self, id: &str) -> Option<Arc<dyn DynDatatable>> {
        self.datatables.get(id).cloned()
    }

    pub fn ids(&self) -> Vec<&str> {
        self.datatables.keys().map(|s| s.as_str()).collect()
    }
}

// ---------------------------------------------------------------------------
// Builder (shared-handle pattern)
// ---------------------------------------------------------------------------

#[derive(Default)]
pub(crate) struct DatatableRegistryBuilder {
    datatables: HashMap<String, Arc<dyn DynDatatable>>,
}

impl DatatableRegistryBuilder {
    pub(crate) fn shared() -> DatatableRegistryHandle {
        Arc::new(Mutex::new(Self::default()))
    }

    pub(crate) fn register<D>(&mut self) -> Result<()>
    where
        D: Datatable,
    {
        let id = D::ID.to_string();
        if self.datatables.contains_key(&id) {
            return Err(Error::message(format!(
                "datatable `{id}` already registered"
            )));
        }
        self.datatables
            .insert(id, Arc::new(DatatableAdapter::<D>::new()));
        Ok(())
    }

    pub(crate) fn freeze_shared(handle: DatatableRegistryHandle) -> DatatableRegistry {
        let mut builder = lock_unpoisoned(&handle, "datatable registry");
        let datatables = std::mem::take(&mut builder.datatables);
        DatatableRegistry { datatables }
    }
}

pub(crate) type DatatableRegistryHandle = Arc<Mutex<DatatableRegistryBuilder>>;
