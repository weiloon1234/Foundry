use std::sync::Arc;

use async_trait::async_trait;

use crate::foundation::Result;
use crate::support::Collection;

use super::ast::DbValue;
use super::model::{Model, PersistedModel};
use super::relation::{AnyRelation, ManyToManyDef, RelationDef};
use super::runtime::QueryExecutor;

// ---------------------------------------------------------------------------
// IntoLoadableRelation — accept RelationDef / ManyToManyDef uniformly
// ---------------------------------------------------------------------------

/// Trait to abstract over relation types so [`ModelCollectionExt::load`] and
/// [`ModelCollectionExt::load_missing`] accept both [`RelationDef`] and
/// [`ManyToManyDef`].
pub trait IntoLoadableRelation<M: Model>: Send + Sync {
    fn into_relation(self) -> AnyRelation<M>;
}

impl<M, To> IntoLoadableRelation<M> for RelationDef<M, To>
where
    M: Model,
    To: Model,
{
    fn into_relation(self) -> AnyRelation<M> {
        Arc::new(self)
    }
}

impl<M, To, Pivot> IntoLoadableRelation<M> for ManyToManyDef<M, To, Pivot>
where
    M: Model,
    To: Model,
    Pivot: Clone + Send + Sync + 'static,
{
    fn into_relation(self) -> AnyRelation<M> {
        Arc::new(self)
    }
}

// ---------------------------------------------------------------------------
// ModelCollectionExt — ORM helpers on Collection<T>
// ---------------------------------------------------------------------------

#[async_trait]
pub trait ModelCollectionExt<T: Model> {
    /// Extract key values from each model using the provided closure.
    fn model_keys(&self, key_fn: impl Fn(&T) -> DbValue) -> Collection<DbValue>
    where
        T: PersistedModel;

    /// Eager-load a relation onto every model in the collection, consuming it
    /// and returning a new collection with the relation data populated.
    async fn load<E>(
        self,
        relation: impl IntoLoadableRelation<T> + 'static,
        executor: &E,
    ) -> Result<Collection<T>>
    where
        E: QueryExecutor;

    /// Eager-load a relation only on models that have not already loaded it.
    async fn load_missing<E>(
        self,
        relation: impl IntoLoadableRelation<T> + 'static,
        executor: &E,
    ) -> Result<Collection<T>>
    where
        E: QueryExecutor;
}

#[async_trait]
impl<T> ModelCollectionExt<T> for Collection<T>
where
    T: Model,
{
    fn model_keys(&self, key_fn: impl Fn(&T) -> DbValue) -> Collection<DbValue>
    where
        T: PersistedModel,
    {
        Collection::from_vec(self.iter().map(key_fn).collect())
    }

    async fn load<E>(
        self,
        relation: impl IntoLoadableRelation<T> + 'static,
        executor: &E,
    ) -> Result<Collection<T>>
    where
        E: QueryExecutor,
    {
        let loader = relation.into_relation();
        let mut items = self.into_vec();
        loader.load(executor, &mut items).await?;
        Ok(Collection::from_vec(items))
    }

    async fn load_missing<E>(
        self,
        relation: impl IntoLoadableRelation<T> + 'static,
        executor: &E,
    ) -> Result<Collection<T>>
    where
        E: QueryExecutor,
    {
        let loader = relation.into_relation();
        let mut items = self.into_vec();
        loader.load_missing(executor, &mut items).await?;
        Ok(Collection::from_vec(items))
    }
}
