use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::auth::Actor;
use crate::foundation::{AppContext, Error, Result};
use crate::support::sync::lock_unpoisoned;

use super::callback::{
    datatable_columns, datatable_default_sort, datatable_mappings, datatable_relation_filters,
};
use super::datatable_trait::Datatable;
use super::request::{DatatableRequest, DatatableSortInput};
use super::response::{DatatableColumnMeta, DatatableExportAccepted, DatatableJsonResponse};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DatatableRelationFilterMeta {
    pub field: String,
    pub relation: String,
    pub aliases: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DatatableDescriptor {
    pub id: String,
    pub columns: Vec<DatatableColumnMeta>,
    pub mappings: Vec<String>,
    pub relation_filters: Vec<DatatableRelationFilterMeta>,
    pub default_sort: Vec<DatatableSortInput>,
}

// ---------------------------------------------------------------------------
// Type-erased datatable interface
// ---------------------------------------------------------------------------

#[async_trait]
pub trait DynDatatable: Send + Sync {
    fn id(&self) -> &str;

    fn descriptor(&self) -> Result<DatatableDescriptor> {
        Ok(DatatableDescriptor {
            id: self.id().to_string(),
            columns: Vec::new(),
            mappings: Vec::new(),
            relation_filters: Vec::new(),
            default_sort: Vec::new(),
        })
    }

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

    fn descriptor(&self) -> Result<DatatableDescriptor> {
        let columns = datatable_columns::<D>()?;
        let mappings = datatable_mappings::<D>()?;
        let relation_filters = datatable_relation_filters::<D>()?;
        let default_sort = datatable_default_sort::<D>()?;

        Ok(DatatableDescriptor {
            id: D::ID.to_string(),
            columns: columns
                .iter()
                .map(DatatableColumnMeta::from_column)
                .collect(),
            mappings: mappings.into_iter().map(|mapping| mapping.name).collect(),
            relation_filters: relation_filters
                .into_iter()
                .map(|filter| {
                    let aliases = filter.aliases().to_vec();
                    DatatableRelationFilterMeta {
                        field: filter.field,
                        relation: filter.relation,
                        aliases,
                    }
                })
                .collect(),
            default_sort: default_sort
                .into_iter()
                .map(|sort| DatatableSortInput {
                    field: sort.field_name,
                    direction: sort.direction,
                })
                .collect(),
        })
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

    pub fn descriptors(&self) -> Result<Vec<DatatableDescriptor>> {
        let mut descriptors = self
            .datatables
            .values()
            .map(|datatable| datatable.descriptor())
            .collect::<Result<Vec<_>>>()?;
        descriptors.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(descriptors)
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
