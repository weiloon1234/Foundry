use std::any::Any;
use std::collections::{BTreeSet, HashMap};
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;

use crate::foundation::{Error, Result};
use crate::logging::{catch_sync_panic, panic_payload_message};

use super::aggregate::decode_aggregate_value;
use super::ast::{
    AggregateNode, ColumnRef, ComparisonOp, Condition, DbValue, Expr, JoinKind, JoinNode, QueryAst,
    RelationKind, RelationNode, SelectItem, SelectNode, TableRef,
};
use super::compiler::PostgresCompiler;
use super::extensions::{register_model_records, AnyModelExtension, MetadataCacheShape};
use super::model::{Column, FromDbValue, Model, ToDbValue};
use super::projection::ProjectionMeta;
use super::query::ModelQuery;
use super::runtime::{DbRecord, QueryExecutor};

const RELATION_GROUP_KEY_ALIAS: &str = "__foundry_relation_key";
const RELATION_AGGREGATE_ALIAS: &str = "__foundry_relation_aggregate";
const PIVOT_ALIAS_PREFIX: &str = "__foundry_pivot_";
const RELATION_KEY_CHUNK_SIZE: usize = 1000;

#[async_trait]
pub trait RelationLoader<From: Send>: Send + Sync {
    /// Returns the relation node metadata.
    fn node(&self) -> RelationNode;
    /// Batch-loads related models onto the given parent slice in-place.
    async fn load(&self, executor: &dyn QueryExecutor, parents: &mut [From]) -> Result<()>;

    /// Like `load`, but skips parents where the relation is already loaded.
    /// Falls back to `load` if no `is_loaded` checker is configured.
    async fn load_missing(&self, executor: &dyn QueryExecutor, parents: &mut [From]) -> Result<()> {
        self.load(executor, parents).await
    }
}

/// Type-erased relation loader: `Arc<dyn RelationLoader<M>>`.
pub type AnyRelation<M> = Arc<dyn RelationLoader<M>>;

#[async_trait]
pub(crate) trait RelationAggregateLoader<From>: Send + Sync {
    fn node(&self) -> RelationNode;
    async fn load(&self, executor: &dyn QueryExecutor, parents: &mut [From]) -> Result<()>;
}

pub(crate) type AnyRelationAggregate<M> = Arc<dyn RelationAggregateLoader<M>>;

type ParentKeyFn<From> = dyn Fn(&From) -> Option<DbValue> + Send + Sync;
type IsLoadedFn<From> = dyn Fn(&From) -> bool + Send + Sync;
type AttachManyFn<From, To> = dyn Fn(&mut From, Vec<To>) + Send + Sync;
type AttachOneFn<From, To> = dyn Fn(&mut From, Option<To>) + Send + Sync;
type AttachAggregateFn<From, Value> = dyn Fn(&mut From, Value) + Send + Sync;
type PivotAttachFn<To, Pivot> = dyn Fn(&mut To, Pivot) + Send + Sync;

trait PivotAttacher<To>: Send + Sync {
    fn select_items(&self, table_name: &str) -> Result<Vec<SelectItem>>;
    fn attach(&self, record: &DbRecord, child: &mut To) -> Result<()>;
}

type AnyPivotAttacher<To> = Arc<dyn PivotAttacher<To>>;

mod belongs_to_foreign_key {
    pub trait Sealed<Key> {}

    impl<Key> Sealed<Key> for Key {}
    impl<Key> Sealed<Key> for Option<Key> {}
}

/// Marker for a `belongs_to` foreign key that is either `Key` or `Option<Key>`.
///
/// This trait is sealed and exists to preserve compile-time key compatibility
/// while allowing nullable foreign-key columns.
#[doc(hidden)]
pub trait BelongsToForeignKey<Key>: belongs_to_foreign_key::Sealed<Key> {}

impl<Key> BelongsToForeignKey<Key> for Key {}
impl<Key> BelongsToForeignKey<Key> for Option<Key> {}

#[derive(Clone)]
pub struct RelationDef<From, To: 'static> {
    name: String,
    kind: RelationKind,
    parent_column: ColumnRef,
    target_column: ColumnRef,
    target_table: &'static super::model::TableMeta<To>,
    parent_key: Arc<ParentKeyFn<From>>,
    attach: RelationAttach<From, To>,
    is_loaded: Option<Arc<IsLoadedFn<From>>>,
    filter: Option<Condition>,
    children: Vec<AnyRelation<To>>,
    child_extensions: Vec<AnyModelExtension<To>>,
    child_aggregates: Vec<AnyRelationAggregate<To>>,
}

#[derive(Clone)]
enum RelationAttach<From, To> {
    Many(Arc<AttachManyFn<From, To>>),
    One(Arc<AttachOneFn<From, To>>),
}

#[derive(Clone)]
pub struct ManyToManyDef<From, To: 'static, Pivot: 'static = ()> {
    name: String,
    parent_column: ColumnRef,
    pivot_table: TableRef,
    pivot_parent_column: ColumnRef,
    pivot_target_column: ColumnRef,
    target_column: ColumnRef,
    target_table: &'static super::model::TableMeta<To>,
    parent_key: Arc<ParentKeyFn<From>>,
    attach: Arc<AttachManyFn<From, To>>,
    is_loaded: Option<Arc<IsLoadedFn<From>>>,
    filter: Option<Condition>,
    children: Vec<AnyRelation<To>>,
    child_extensions: Vec<AnyModelExtension<To>>,
    child_aggregates: Vec<AnyRelationAggregate<To>>,
    pivot_attacher: Option<AnyPivotAttacher<To>>,
    _pivot: PhantomData<fn() -> Pivot>,
}

#[derive(Clone)]
pub struct RelationAggregateDef<From, Value: 'static> {
    loader: AnyRelationAggregate<From>,
    _marker: PhantomData<fn() -> Value>,
}

impl<From, Value: 'static> RelationAggregateDef<From, Value> {
    pub(crate) fn new(loader: AnyRelationAggregate<From>) -> Self {
        Self {
            loader,
            _marker: PhantomData,
        }
    }

    pub(crate) fn node(&self) -> RelationNode {
        self.loader.node()
    }

    pub(crate) fn into_loader(self) -> AnyRelationAggregate<From> {
        self.loader
    }
}

#[derive(Clone)]
enum AggregateKind {
    CountAll,
    CountDistinct(ColumnRef),
    Sum(ColumnRef),
    Avg(ColumnRef),
    Min(ColumnRef),
    Max(ColumnRef),
}

impl AggregateKind {
    fn node(self) -> AggregateNode {
        match self {
            Self::CountAll => AggregateNode::count_all(RELATION_AGGREGATE_ALIAS),
            Self::CountDistinct(column) => {
                AggregateNode::count_distinct(Expr::column(column), RELATION_AGGREGATE_ALIAS)
            }
            Self::Sum(column) => AggregateNode::sum(Expr::column(column), RELATION_AGGREGATE_ALIAS),
            Self::Avg(column) => AggregateNode::avg(Expr::column(column), RELATION_AGGREGATE_ALIAS),
            Self::Min(column) => AggregateNode::min(Expr::column(column), RELATION_AGGREGATE_ALIAS),
            Self::Max(column) => AggregateNode::max(Expr::column(column), RELATION_AGGREGATE_ALIAS),
        }
    }
}

#[derive(Clone)]
struct TypedPivotAttacher<To, Pivot: 'static> {
    meta: &'static ProjectionMeta<Pivot>,
    attach: Arc<PivotAttachFn<To, Pivot>>,
}

impl<To, Pivot> PivotAttacher<To> for TypedPivotAttacher<To, Pivot>
where
    Pivot: Clone + Send + Sync + 'static,
    To: Send + Sync + 'static,
{
    fn select_items(&self, table_name: &str) -> Result<Vec<SelectItem>> {
        self.meta
            .fields()
            .iter()
            .map(|field| {
                let source_column = field.source_column.ok_or_else(|| {
                    Error::message("pivot projection field requires a source column")
                })?;
                Ok(
                    SelectItem::new(ColumnRef::new(table_name, source_column).typed(field.db_type))
                        .aliased(format!("{PIVOT_ALIAS_PREFIX}{}", field.alias)),
                )
            })
            .collect()
    }

    fn attach(&self, record: &DbRecord, child: &mut To) -> Result<()> {
        let mut pivot_record = DbRecord::new();
        for field in self.meta.fields() {
            let key = format!("{PIVOT_ALIAS_PREFIX}{}", field.alias);
            let value = record
                .get(&key)
                .ok_or_else(|| Error::message(format!("missing pivot field `{key}` in record")))?;
            pivot_record.insert(field.alias.to_string(), value.clone());
        }
        let pivot = self.meta.hydrate_record(&pivot_record)?;
        catch_sync_panic(|| (self.attach)(child, pivot))
            .map_err(|panic| relation_callback_panic_error("pivot attach callback", panic))?;
        Ok(())
    }
}

impl<From, To> RelationDef<From, To>
where
    From: Model,
    To: Model,
{
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with<Child>(mut self, child: RelationDef<To, Child>) -> Self
    where
        Child: Model,
    {
        self.children.push(Arc::new(child));
        self
    }

    pub fn with_many_to_many<Child, Pivot>(mut self, child: ManyToManyDef<To, Child, Pivot>) -> Self
    where
        Child: Model,
        Pivot: Clone + Send + Sync + 'static,
    {
        self.children.push(Arc::new(child));
        self
    }

    pub fn with_attachments(mut self, collection: impl Into<String>) -> Self
    where
        To: crate::attachments::HasAttachments,
    {
        self.child_extensions
            .push(crate::attachments::attachment_extension_loader(
                collection.into(),
            ));
        self
    }

    pub fn with_meta(mut self, key: impl Into<String>) -> Self
    where
        To: crate::metadata::HasMetadata,
    {
        self.child_extensions
            .push(crate::metadata::metadata_extension_loader(
                MetadataCacheShape::Key(key.into()),
            ));
        self
    }

    pub fn with_metadata(mut self) -> Self
    where
        To: crate::metadata::HasMetadata,
    {
        self.child_extensions
            .push(crate::metadata::metadata_extension_loader(
                MetadataCacheShape::All,
            ));
        self
    }

    pub fn with_translated_field(mut self, field: impl Into<String>) -> Self
    where
        To: crate::translations::HasTranslations,
    {
        self.child_extensions
            .push(crate::translations::translated_field_extension_loader(
                field.into(),
            ));
        self
    }

    pub fn with_translations_for(mut self, locale: impl Into<String>) -> Self
    where
        To: crate::translations::HasTranslations,
    {
        self.child_extensions
            .push(crate::translations::translations_for_extension_loader(
                locale.into(),
            ));
        self
    }

    pub fn with_all_translations(mut self) -> Self
    where
        To: crate::translations::HasTranslations,
    {
        self.child_extensions
            .push(crate::translations::all_translations_extension_loader());
        self
    }

    pub fn with_aggregate<Value>(mut self, aggregate: RelationAggregateDef<To, Value>) -> Self {
        self.child_aggregates.push(aggregate.into_loader());
        self
    }

    pub fn where_(mut self, condition: Condition) -> Self {
        self.filter = merge_condition(self.filter.take(), condition);
        self
    }

    pub fn is_loaded(mut self, f: impl Fn(&From) -> bool + Send + Sync + 'static) -> Self {
        self.is_loaded = Some(Arc::new(f));
        self
    }

    pub fn count(self, attach: fn(&mut From, i64)) -> RelationAggregateDef<From, i64> {
        RelationAggregateDef::new(Arc::new(ScalarRelationAggregate {
            relation: self,
            kind: AggregateKind::CountAll,
            attach: Arc::new(attach),
            _marker: PhantomData,
        }))
    }

    pub fn count_distinct<Value>(
        self,
        column: Column<To, Value>,
        attach: fn(&mut From, i64),
    ) -> RelationAggregateDef<From, i64> {
        RelationAggregateDef::new(Arc::new(ScalarRelationAggregate {
            relation: self,
            kind: AggregateKind::CountDistinct(column.column_ref()),
            attach: Arc::new(attach),
            _marker: PhantomData,
        }))
    }

    pub fn sum<Value>(
        self,
        column: Column<To, Value>,
        attach: fn(&mut From, Option<Value>),
    ) -> RelationAggregateDef<From, Option<Value>>
    where
        Value: ToDbValue + FromDbValue + Send + Sync + 'static,
    {
        RelationAggregateDef::new(Arc::new(ScalarRelationAggregate {
            relation: self,
            kind: AggregateKind::Sum(column.column_ref()),
            attach: Arc::new(attach),
            _marker: PhantomData,
        }))
    }

    pub fn avg<Value>(
        self,
        column: Column<To, Value>,
        attach: fn(&mut From, Option<Value>),
    ) -> RelationAggregateDef<From, Option<Value>>
    where
        Value: ToDbValue + FromDbValue + Send + Sync + 'static,
    {
        RelationAggregateDef::new(Arc::new(ScalarRelationAggregate {
            relation: self,
            kind: AggregateKind::Avg(column.column_ref()),
            attach: Arc::new(attach),
            _marker: PhantomData,
        }))
    }

    pub fn min<Value>(
        self,
        column: Column<To, Value>,
        attach: fn(&mut From, Option<Value>),
    ) -> RelationAggregateDef<From, Option<Value>>
    where
        Value: ToDbValue + FromDbValue + Send + Sync + 'static,
    {
        RelationAggregateDef::new(Arc::new(ScalarRelationAggregate {
            relation: self,
            kind: AggregateKind::Min(column.column_ref()),
            attach: Arc::new(attach),
            _marker: PhantomData,
        }))
    }

    pub fn max<Value>(
        self,
        column: Column<To, Value>,
        attach: fn(&mut From, Option<Value>),
    ) -> RelationAggregateDef<From, Option<Value>>
    where
        Value: ToDbValue + FromDbValue + Send + Sync + 'static,
    {
        RelationAggregateDef::new(Arc::new(ScalarRelationAggregate {
            relation: self,
            kind: AggregateKind::Max(column.column_ref()),
            attach: Arc::new(attach),
            _marker: PhantomData,
        }))
    }

    pub fn node(&self) -> RelationNode {
        RelationNode {
            name: self.name.clone(),
            kind: self.kind,
            target: self.target_table.table_ref(),
            local_key: self.parent_column.clone(),
            foreign_key: self.target_column.clone(),
            pivot: None,
            filters: self.filter.clone(),
            children: self
                .children
                .iter()
                .map(|child| child.node())
                .chain(
                    self.child_aggregates
                        .iter()
                        .map(|aggregate| aggregate.node()),
                )
                .collect(),
            aggregates: Vec::new(),
        }
    }

    pub(crate) fn scoped_with_filter(mut self, filter: Option<Condition>) -> Self {
        if let Some(filter) = filter {
            self.filter = merge_condition(self.filter.take(), filter);
        }
        self
    }

    pub(crate) fn exists_condition(&self) -> Condition {
        let mut exists_select = SelectNode::from(self.target_table.table_ref());
        exists_select.columns = vec![SelectItem::new(Expr::raw("1"))];
        let condition = Condition::compare(
            Expr::column(self.target_column.clone()),
            ComparisonOp::Eq,
            Expr::column(self.parent_column.clone()),
        );
        exists_select.condition = Some(match self.filter.clone() {
            Some(filter) => Condition::and([filter, condition]),
            None => condition,
        });
        Condition::exists(QueryAst::select(exists_select))
    }
}

impl<From, To, Pivot> ManyToManyDef<From, To, Pivot>
where
    From: Model,
    To: Model,
    Pivot: Clone + Send + Sync + 'static,
{
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with<Child>(mut self, child: RelationDef<To, Child>) -> Self
    where
        Child: Model,
    {
        self.children.push(Arc::new(child));
        self
    }

    pub fn with_many_to_many<Child, ChildPivot>(
        mut self,
        child: ManyToManyDef<To, Child, ChildPivot>,
    ) -> Self
    where
        Child: Model,
        ChildPivot: Clone + Send + Sync + 'static,
    {
        self.children.push(Arc::new(child));
        self
    }

    pub fn with_attachments(mut self, collection: impl Into<String>) -> Self
    where
        To: crate::attachments::HasAttachments,
    {
        self.child_extensions
            .push(crate::attachments::attachment_extension_loader(
                collection.into(),
            ));
        self
    }

    pub fn with_meta(mut self, key: impl Into<String>) -> Self
    where
        To: crate::metadata::HasMetadata,
    {
        self.child_extensions
            .push(crate::metadata::metadata_extension_loader(
                MetadataCacheShape::Key(key.into()),
            ));
        self
    }

    pub fn with_metadata(mut self) -> Self
    where
        To: crate::metadata::HasMetadata,
    {
        self.child_extensions
            .push(crate::metadata::metadata_extension_loader(
                MetadataCacheShape::All,
            ));
        self
    }

    pub fn with_translated_field(mut self, field: impl Into<String>) -> Self
    where
        To: crate::translations::HasTranslations,
    {
        self.child_extensions
            .push(crate::translations::translated_field_extension_loader(
                field.into(),
            ));
        self
    }

    pub fn with_translations_for(mut self, locale: impl Into<String>) -> Self
    where
        To: crate::translations::HasTranslations,
    {
        self.child_extensions
            .push(crate::translations::translations_for_extension_loader(
                locale.into(),
            ));
        self
    }

    pub fn with_all_translations(mut self) -> Self
    where
        To: crate::translations::HasTranslations,
    {
        self.child_extensions
            .push(crate::translations::all_translations_extension_loader());
        self
    }

    pub fn with_aggregate<Value>(mut self, aggregate: RelationAggregateDef<To, Value>) -> Self {
        self.child_aggregates.push(aggregate.into_loader());
        self
    }

    pub fn where_(mut self, condition: Condition) -> Self {
        self.filter = merge_condition(self.filter.take(), condition);
        self
    }

    pub fn is_loaded(mut self, f: impl Fn(&From) -> bool + Send + Sync + 'static) -> Self {
        self.is_loaded = Some(Arc::new(f));
        self
    }

    pub fn with_pivot<NewPivot>(
        self,
        meta: &'static ProjectionMeta<NewPivot>,
        attach: fn(&mut To, NewPivot),
    ) -> ManyToManyDef<From, To, NewPivot>
    where
        NewPivot: Clone + Send + Sync + 'static,
    {
        ManyToManyDef {
            name: self.name,
            parent_column: self.parent_column,
            pivot_table: self.pivot_table,
            pivot_parent_column: self.pivot_parent_column,
            pivot_target_column: self.pivot_target_column,
            target_column: self.target_column,
            target_table: self.target_table,
            parent_key: self.parent_key,
            attach: self.attach,
            is_loaded: self.is_loaded,
            filter: self.filter,
            children: self.children,
            child_extensions: self.child_extensions,
            child_aggregates: self.child_aggregates,
            pivot_attacher: Some(Arc::new(TypedPivotAttacher {
                meta,
                attach: Arc::new(attach),
            })),
            _pivot: PhantomData,
        }
    }

    pub fn count(self, attach: fn(&mut From, i64)) -> RelationAggregateDef<From, i64> {
        RelationAggregateDef::new(Arc::new(ManyToManyAggregate {
            relation: self,
            kind: AggregateKind::CountAll,
            attach: Arc::new(attach),
            _marker: PhantomData,
        }))
    }

    pub fn count_distinct<Value>(
        self,
        column: Column<To, Value>,
        attach: fn(&mut From, i64),
    ) -> RelationAggregateDef<From, i64> {
        RelationAggregateDef::new(Arc::new(ManyToManyAggregate {
            relation: self,
            kind: AggregateKind::CountDistinct(column.column_ref()),
            attach: Arc::new(attach),
            _marker: PhantomData,
        }))
    }

    pub fn sum<Value>(
        self,
        column: Column<To, Value>,
        attach: fn(&mut From, Option<Value>),
    ) -> RelationAggregateDef<From, Option<Value>>
    where
        Value: ToDbValue + FromDbValue + Send + Sync + 'static,
    {
        RelationAggregateDef::new(Arc::new(ManyToManyAggregate {
            relation: self,
            kind: AggregateKind::Sum(column.column_ref()),
            attach: Arc::new(attach),
            _marker: PhantomData,
        }))
    }

    pub fn avg<Value>(
        self,
        column: Column<To, Value>,
        attach: fn(&mut From, Option<Value>),
    ) -> RelationAggregateDef<From, Option<Value>>
    where
        Value: ToDbValue + FromDbValue + Send + Sync + 'static,
    {
        RelationAggregateDef::new(Arc::new(ManyToManyAggregate {
            relation: self,
            kind: AggregateKind::Avg(column.column_ref()),
            attach: Arc::new(attach),
            _marker: PhantomData,
        }))
    }

    pub fn min<Value>(
        self,
        column: Column<To, Value>,
        attach: fn(&mut From, Option<Value>),
    ) -> RelationAggregateDef<From, Option<Value>>
    where
        Value: ToDbValue + FromDbValue + Send + Sync + 'static,
    {
        RelationAggregateDef::new(Arc::new(ManyToManyAggregate {
            relation: self,
            kind: AggregateKind::Min(column.column_ref()),
            attach: Arc::new(attach),
            _marker: PhantomData,
        }))
    }

    pub fn max<Value>(
        self,
        column: Column<To, Value>,
        attach: fn(&mut From, Option<Value>),
    ) -> RelationAggregateDef<From, Option<Value>>
    where
        Value: ToDbValue + FromDbValue + Send + Sync + 'static,
    {
        RelationAggregateDef::new(Arc::new(ManyToManyAggregate {
            relation: self,
            kind: AggregateKind::Max(column.column_ref()),
            attach: Arc::new(attach),
            _marker: PhantomData,
        }))
    }

    pub fn node(&self) -> RelationNode {
        RelationNode {
            name: self.name.clone(),
            kind: RelationKind::ManyToMany,
            target: self.target_table.table_ref(),
            local_key: self.parent_column.clone(),
            foreign_key: self.target_column.clone(),
            pivot: Some(super::ast::PivotNode {
                table: self.pivot_table.clone(),
                local_key: self.pivot_parent_column.clone(),
                foreign_key: self.pivot_target_column.clone(),
            }),
            filters: self.filter.clone(),
            children: self
                .children
                .iter()
                .map(|child| child.node())
                .chain(
                    self.child_aggregates
                        .iter()
                        .map(|aggregate| aggregate.node()),
                )
                .collect(),
            aggregates: Vec::new(),
        }
    }

    pub(crate) fn scoped_with_filter(mut self, filter: Option<Condition>) -> Self {
        if let Some(filter) = filter {
            self.filter = merge_condition(self.filter.take(), filter);
        }
        self
    }

    pub(crate) fn exists_condition(&self) -> Condition {
        let mut exists_select = SelectNode::from(self.target_table.table_ref());
        exists_select.columns = vec![SelectItem::new(Expr::raw("1"))];
        exists_select.joins.push(JoinNode {
            kind: JoinKind::Inner,
            table: self.pivot_table.clone().into(),
            lateral: false,
            on: Some(Condition::compare(
                Expr::column(self.target_column.clone()),
                ComparisonOp::Eq,
                Expr::column(self.pivot_target_column.clone()),
            )),
        });
        let condition = Condition::compare(
            Expr::column(self.pivot_parent_column.clone()),
            ComparisonOp::Eq,
            Expr::column(self.parent_column.clone()),
        );
        exists_select.condition = Some(match self.filter.clone() {
            Some(filter) => Condition::and([filter, condition]),
            None => condition,
        });
        Condition::exists(QueryAst::select(exists_select))
    }
}

#[async_trait]
impl<From, To> RelationLoader<From> for RelationDef<From, To>
where
    From: Model,
    To: Model,
{
    fn node(&self) -> RelationNode {
        self.node()
    }

    async fn load(&self, executor: &dyn QueryExecutor, parents: &mut [From]) -> Result<()> {
        let keys = collect_relation_keys(parents, |parent| relation_parent_key(self, parent))?;
        let child_entries = fetch_relation_child_entries(executor, self, keys).await?;
        let mut grouped = group_relation_entries(child_entries, &self.target_column)
            .map_err(|error| relation_load_error(self, "group related models", error))?;
        let mut remaining =
            count_parent_relation_keys(parents, |parent| relation_parent_key(self, parent))?;

        for parent in parents.iter_mut() {
            let values = relation_parent_key(self, parent)?
                .map(|key| take_grouped_values(&mut grouped, &mut remaining, &key.relation_key()))
                .unwrap_or_default();
            attach_relation_values(self, parent, values)?;
        }

        Ok(())
    }

    async fn load_missing(&self, executor: &dyn QueryExecutor, parents: &mut [From]) -> Result<()> {
        let is_loaded_fn = match &self.is_loaded {
            Some(f) => f,
            None => return self.load(executor, parents).await,
        };

        // Find indices of parents that need loading
        let mut unloaded_indices = Vec::new();
        for (index, parent) in parents.iter().enumerate() {
            if !relation_is_loaded(self, is_loaded_fn, parent)? {
                unloaded_indices.push(index);
            }
        }

        if unloaded_indices.is_empty() {
            return Ok(());
        }

        // Collect keys only from unloaded parents (with dedup)
        let mut keys = Vec::new();
        let mut seen = BTreeSet::new();
        for &i in &unloaded_indices {
            if let Some(key) = relation_parent_key(self, &parents[i])? {
                if seen.insert(key.relation_key()) {
                    keys.push(key);
                }
            }
        }

        let child_entries = fetch_relation_child_entries(executor, self, keys).await?;
        let mut grouped = group_relation_entries(child_entries, &self.target_column)
            .map_err(|error| relation_load_error(self, "group related models", error))?;
        let mut remaining = count_parent_relation_keys_for_indices(
            parents,
            |parent| relation_parent_key(self, parent),
            &unloaded_indices,
        )?;

        // Attach only to unloaded parents
        for &i in &unloaded_indices {
            let parent = &mut parents[i];
            let values = relation_parent_key(self, parent)?
                .map(|key| take_grouped_values(&mut grouped, &mut remaining, &key.relation_key()))
                .unwrap_or_default();
            attach_relation_values(self, parent, values)?;
        }

        Ok(())
    }
}

#[async_trait]
impl<From, To, Pivot> RelationLoader<From> for ManyToManyDef<From, To, Pivot>
where
    From: Model,
    To: Model,
    Pivot: Clone + Send + Sync + 'static,
{
    fn node(&self) -> RelationNode {
        self.node()
    }

    async fn load(&self, executor: &dyn QueryExecutor, parents: &mut [From]) -> Result<()> {
        let keys = collect_relation_keys(parents, |parent| many_to_many_parent_key(self, parent))?;
        if keys.is_empty() {
            for parent in parents.iter_mut() {
                attach_many_to_many_values(self, parent, Vec::new())?;
            }
            return Ok(());
        }

        let records = fetch_many_to_many_records(executor, self, keys)
            .await
            .map_err(|error| many_to_many_load_error(self, "fetch related records", error))?;
        let mut models = hydrate_many_to_many_models(self, &records)?;

        if let Some(pivot_attacher) = &self.pivot_attacher {
            for (record, model) in records.iter().zip(models.iter_mut()) {
                pivot_attacher.attach(record, model).map_err(|error| {
                    many_to_many_load_error(self, "hydrate pivot projection", error)
                })?;
            }
        }

        register_model_records(self.target_table, &records);

        for extension in &self.child_extensions {
            extension.load(executor, &models).await.map_err(|error| {
                many_to_many_load_error(self, "load child model extensions", error)
            })?;
        }

        for child in &self.children {
            child
                .load(executor, &mut models)
                .await
                .map_err(|error| many_to_many_load_error(self, "load nested relations", error))?;
        }

        for aggregate in &self.child_aggregates {
            aggregate
                .load(executor, &mut models)
                .await
                .map_err(|error| {
                    many_to_many_load_error(self, "load nested relation aggregates", error)
                })?;
        }

        let mut grouped: HashMap<String, Vec<To>> = HashMap::new();
        for (record, model) in records.into_iter().zip(models) {
            let key = record
                .get(RELATION_GROUP_KEY_ALIAS)
                .ok_or_else(|| Error::message("missing many-to-many group key in record"))
                .map_err(|error| many_to_many_load_error(self, "group related models", error))?
                .relation_key();
            grouped.entry(key).or_default().push(model);
        }
        let mut remaining =
            count_parent_relation_keys(parents, |parent| many_to_many_parent_key(self, parent))?;

        for parent in parents.iter_mut() {
            let children = many_to_many_parent_key(self, parent)?
                .map(|key| take_grouped_values(&mut grouped, &mut remaining, &key.relation_key()))
                .unwrap_or_default();
            attach_many_to_many_values(self, parent, children)?;
        }

        Ok(())
    }

    async fn load_missing(&self, executor: &dyn QueryExecutor, parents: &mut [From]) -> Result<()> {
        let is_loaded_fn = match &self.is_loaded {
            Some(f) => f,
            None => return self.load(executor, parents).await,
        };

        let mut unloaded_indices = Vec::new();
        for (index, parent) in parents.iter().enumerate() {
            if !many_to_many_is_loaded(self, is_loaded_fn, parent)? {
                unloaded_indices.push(index);
            }
        }

        if unloaded_indices.is_empty() {
            return Ok(());
        }

        // Collect keys only from unloaded parents
        let mut keys = Vec::new();
        let mut seen = BTreeSet::new();
        for &i in &unloaded_indices {
            if let Some(key) = many_to_many_parent_key(self, &parents[i])? {
                if seen.insert(key.relation_key()) {
                    keys.push(key);
                }
            }
        }

        if keys.is_empty() {
            for &i in &unloaded_indices {
                let parent = &mut parents[i];
                attach_many_to_many_values(self, parent, Vec::new())?;
            }
            return Ok(());
        }

        let records = fetch_many_to_many_records(executor, self, keys)
            .await
            .map_err(|error| many_to_many_load_error(self, "fetch related records", error))?;
        let mut models = hydrate_many_to_many_models(self, &records)?;

        if let Some(pivot_attacher) = &self.pivot_attacher {
            for (record, model) in records.iter().zip(models.iter_mut()) {
                pivot_attacher.attach(record, model).map_err(|error| {
                    many_to_many_load_error(self, "hydrate pivot projection", error)
                })?;
            }
        }

        register_model_records(self.target_table, &records);

        for extension in &self.child_extensions {
            extension.load(executor, &models).await.map_err(|error| {
                many_to_many_load_error(self, "load child model extensions", error)
            })?;
        }

        for child in &self.children {
            child
                .load(executor, &mut models)
                .await
                .map_err(|error| many_to_many_load_error(self, "load nested relations", error))?;
        }
        for aggregate in &self.child_aggregates {
            aggregate
                .load(executor, &mut models)
                .await
                .map_err(|error| {
                    many_to_many_load_error(self, "load nested relation aggregates", error)
                })?;
        }

        let mut grouped: HashMap<String, Vec<To>> = HashMap::new();
        for (record, model) in records.into_iter().zip(models) {
            let key = record
                .get(RELATION_GROUP_KEY_ALIAS)
                .ok_or_else(|| Error::message("missing many-to-many group key in record"))
                .map_err(|error| many_to_many_load_error(self, "group related models", error))?
                .relation_key();
            grouped.entry(key).or_default().push(model);
        }
        let mut remaining = count_parent_relation_keys_for_indices(
            parents,
            |parent| many_to_many_parent_key(self, parent),
            &unloaded_indices,
        )?;

        // Attach only to unloaded parents
        for &i in &unloaded_indices {
            let parent = &mut parents[i];
            let children = many_to_many_parent_key(self, parent)?
                .map(|key| take_grouped_values(&mut grouped, &mut remaining, &key.relation_key()))
                .unwrap_or_default();
            attach_many_to_many_values(self, parent, children)?;
        }

        Ok(())
    }
}

#[derive(Clone)]
struct ScalarRelationAggregate<From, To: 'static, Value: 'static> {
    relation: RelationDef<From, To>,
    kind: AggregateKind,
    attach: Arc<AttachAggregateFn<From, Value>>,
    _marker: PhantomData<fn() -> Value>,
}

#[async_trait]
impl<From, To> RelationAggregateLoader<From> for ScalarRelationAggregate<From, To, i64>
where
    From: Model,
    To: Model,
{
    fn node(&self) -> RelationNode {
        let mut node = self.relation.node();
        node.aggregates.push(self.kind.clone().node());
        node
    }

    async fn load(&self, executor: &dyn QueryExecutor, parents: &mut [From]) -> Result<()> {
        let keys = collect_relation_keys(parents, |parent| {
            relation_parent_key(&self.relation, parent)
        })?;
        let grouped =
            execute_relation_aggregate_query(executor, &self.relation, self.kind.clone(), keys)
                .await
                .map_err(|error| {
                    relation_load_error(&self.relation, "load aggregate values", error)
                })?;
        for parent in parents.iter_mut() {
            let value = relation_parent_key(&self.relation, parent)?
                .and_then(|key| grouped.get(&key.relation_key()))
                .and_then(|record| record.decode::<i64>(RELATION_AGGREGATE_ALIAS).ok())
                .unwrap_or(0);
            attach_relation_aggregate_value(&self.relation, &self.attach, parent, value)?;
        }
        Ok(())
    }
}

#[async_trait]
impl<From, To, Value> RelationAggregateLoader<From>
    for ScalarRelationAggregate<From, To, Option<Value>>
where
    From: Model,
    To: Model,
    Value: ToDbValue + FromDbValue + Send + Sync + 'static,
{
    fn node(&self) -> RelationNode {
        let mut node = self.relation.node();
        node.aggregates.push(self.kind.clone().node());
        node
    }

    async fn load(&self, executor: &dyn QueryExecutor, parents: &mut [From]) -> Result<()> {
        let keys = collect_relation_keys(parents, |parent| {
            relation_parent_key(&self.relation, parent)
        })?;
        let grouped =
            execute_relation_aggregate_query(executor, &self.relation, self.kind.clone(), keys)
                .await
                .map_err(|error| {
                    relation_load_error(&self.relation, "load aggregate values", error)
                })?;
        for parent in parents.iter_mut() {
            let value = relation_parent_key(&self.relation, parent)?
                .and_then(|key| grouped.get(&key.relation_key()))
                .map(|record| {
                    decode_aggregate_value::<Option<Value>>(record, RELATION_AGGREGATE_ALIAS)
                })
                .transpose()
                .map_err(|error| {
                    relation_load_error(&self.relation, "decode aggregate value", error)
                })?
                .flatten();
            attach_relation_aggregate_value(&self.relation, &self.attach, parent, value)?;
        }
        Ok(())
    }
}

#[derive(Clone)]
struct ManyToManyAggregate<From, To: 'static, Pivot: 'static, Value: 'static> {
    relation: ManyToManyDef<From, To, Pivot>,
    kind: AggregateKind,
    attach: Arc<AttachAggregateFn<From, Value>>,
    _marker: PhantomData<fn() -> Value>,
}

#[async_trait]
impl<From, To, Pivot> RelationAggregateLoader<From> for ManyToManyAggregate<From, To, Pivot, i64>
where
    From: Model,
    To: Model,
    Pivot: Clone + Send + Sync + 'static,
{
    fn node(&self) -> RelationNode {
        let mut node = self.relation.node();
        node.aggregates.push(self.kind.clone().node());
        node
    }

    async fn load(&self, executor: &dyn QueryExecutor, parents: &mut [From]) -> Result<()> {
        let keys = collect_relation_keys(parents, |parent| {
            many_to_many_parent_key(&self.relation, parent)
        })?;
        let grouped =
            execute_many_to_many_aggregate_query(executor, &self.relation, self.kind.clone(), keys)
                .await
                .map_err(|error| {
                    many_to_many_load_error(&self.relation, "load aggregate values", error)
                })?;
        for parent in parents.iter_mut() {
            let value = many_to_many_parent_key(&self.relation, parent)?
                .and_then(|key| grouped.get(&key.relation_key()))
                .and_then(|record| record.decode::<i64>(RELATION_AGGREGATE_ALIAS).ok())
                .unwrap_or(0);
            attach_many_to_many_aggregate_value(&self.relation, &self.attach, parent, value)?;
        }
        Ok(())
    }
}

#[async_trait]
impl<From, To, Pivot, Value> RelationAggregateLoader<From>
    for ManyToManyAggregate<From, To, Pivot, Option<Value>>
where
    From: Model,
    To: Model,
    Pivot: Clone + Send + Sync + 'static,
    Value: ToDbValue + FromDbValue + Send + Sync + 'static,
{
    fn node(&self) -> RelationNode {
        let mut node = self.relation.node();
        node.aggregates.push(self.kind.clone().node());
        node
    }

    async fn load(&self, executor: &dyn QueryExecutor, parents: &mut [From]) -> Result<()> {
        let keys = collect_relation_keys(parents, |parent| {
            many_to_many_parent_key(&self.relation, parent)
        })?;
        let grouped =
            execute_many_to_many_aggregate_query(executor, &self.relation, self.kind.clone(), keys)
                .await
                .map_err(|error| {
                    many_to_many_load_error(&self.relation, "load aggregate values", error)
                })?;
        for parent in parents.iter_mut() {
            let value = many_to_many_parent_key(&self.relation, parent)?
                .and_then(|key| grouped.get(&key.relation_key()))
                .map(|record| {
                    decode_aggregate_value::<Option<Value>>(record, RELATION_AGGREGATE_ALIAS)
                })
                .transpose()
                .map_err(|error| {
                    many_to_many_load_error(&self.relation, "decode aggregate value", error)
                })?
                .flatten();
            attach_many_to_many_aggregate_value(&self.relation, &self.attach, parent, value)?;
        }
        Ok(())
    }
}

pub fn has_many<From, To, Key>(
    local_key: Column<From, Key>,
    foreign_key: Column<To, Key>,
    parent_key: fn(&From) -> Key,
    attach: fn(&mut From, Vec<To>),
) -> RelationDef<From, To>
where
    From: Model,
    To: Model,
    Key: ToDbValue + 'static,
{
    RelationDef {
        name: infer_collection_relation_name(To::table_meta().name()),
        kind: RelationKind::HasMany,
        parent_column: local_key.column_ref(),
        target_column: foreign_key.column_ref(),
        target_table: To::table_meta(),
        parent_key: Arc::new(move |parent| Some(parent_key(parent).to_db_value())),
        attach: RelationAttach::Many(Arc::new(attach)),
        is_loaded: None,
        filter: None,
        children: Vec::new(),
        child_extensions: Vec::new(),
        child_aggregates: Vec::new(),
    }
}

pub fn has_one<From, To, Key>(
    local_key: Column<From, Key>,
    foreign_key: Column<To, Key>,
    parent_key: fn(&From) -> Key,
    attach: fn(&mut From, Option<To>),
) -> RelationDef<From, To>
where
    From: Model,
    To: Model,
    Key: ToDbValue + 'static,
{
    RelationDef {
        name: infer_singular_relation_name(To::table_meta().name()),
        kind: RelationKind::HasOne,
        parent_column: local_key.column_ref(),
        target_column: foreign_key.column_ref(),
        target_table: To::table_meta(),
        parent_key: Arc::new(move |parent| Some(parent_key(parent).to_db_value())),
        attach: RelationAttach::One(Arc::new(attach)),
        is_loaded: None,
        filter: None,
        children: Vec::new(),
        child_extensions: Vec::new(),
        child_aggregates: Vec::new(),
    }
}

pub fn belongs_to<From, To, ForeignKey, Key>(
    foreign_key: Column<From, ForeignKey>,
    owner_key: Column<To, Key>,
    parent_key: fn(&From) -> Option<Key>,
    attach: fn(&mut From, Option<To>),
) -> RelationDef<From, To>
where
    From: Model,
    To: Model,
    ForeignKey: BelongsToForeignKey<Key> + 'static,
    Key: ToDbValue + 'static,
{
    RelationDef {
        name: infer_singular_relation_name(To::table_meta().name()),
        kind: RelationKind::BelongsTo,
        parent_column: foreign_key.column_ref(),
        target_column: owner_key.column_ref(),
        target_table: To::table_meta(),
        parent_key: Arc::new(move |parent| parent_key(parent).map(ToDbValue::to_db_value)),
        attach: RelationAttach::One(Arc::new(attach)),
        is_loaded: None,
        filter: None,
        children: Vec::new(),
        child_extensions: Vec::new(),
        child_aggregates: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn many_to_many<From, To, Pivot, LocalKey, TargetKey>(
    local_key: Column<From, LocalKey>,
    pivot_table: &'static str,
    pivot_local_key: &'static str,
    pivot_related_key: &'static str,
    target_key: Column<To, TargetKey>,
    parent_key: fn(&From) -> LocalKey,
    attach: fn(&mut From, Vec<To>),
) -> ManyToManyDef<From, To, Pivot>
where
    From: Model,
    To: Model,
    LocalKey: ToDbValue + 'static,
    TargetKey: 'static,
    Pivot: Clone + Send + Sync + 'static,
{
    ManyToManyDef {
        name: infer_collection_relation_name(To::table_meta().name()),
        parent_column: local_key.column_ref(),
        pivot_table: TableRef::new(pivot_table),
        pivot_parent_column: ColumnRef::new(pivot_table, pivot_local_key)
            .typed(local_key.db_type()),
        pivot_target_column: ColumnRef::new(pivot_table, pivot_related_key)
            .typed(target_key.db_type()),
        target_column: target_key.column_ref(),
        target_table: To::table_meta(),
        parent_key: Arc::new(move |parent| Some(parent_key(parent).to_db_value())),
        attach: Arc::new(attach),
        is_loaded: None,
        filter: None,
        children: Vec::new(),
        child_extensions: Vec::new(),
        child_aggregates: Vec::new(),
        pivot_attacher: None,
        _pivot: PhantomData,
    }
}

fn infer_collection_relation_name(table_name: &str) -> String {
    relation_basename(table_name).to_string()
}

fn infer_singular_relation_name(table_name: &str) -> String {
    singularize_relation_name(relation_basename(table_name))
}

fn relation_basename(table_name: &str) -> &str {
    table_name.rsplit('.').next().unwrap_or(table_name)
}

fn singularize_relation_name(name: &str) -> String {
    if let Some(stem) = name.strip_suffix("ies") {
        return format!("{stem}y");
    }

    for suffix in ["sses", "shes", "ches", "xes", "zes"] {
        if let Some(stem) = name.strip_suffix(suffix) {
            return format!("{stem}{}", &suffix[..suffix.len() - 2]);
        }
    }

    if let Some(stem) = name.strip_suffix('s') {
        if !stem.ends_with('s') {
            return stem.to_string();
        }
    }

    name.to_string()
}

async fn fetch_relation_child_entries<From, To>(
    executor: &dyn QueryExecutor,
    relation: &RelationDef<From, To>,
    keys: Vec<DbValue>,
) -> Result<Vec<(DbRecord, To)>>
where
    From: Model,
    To: Model,
{
    if keys.is_empty() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for chunk in keys.chunks(RELATION_KEY_CHUNK_SIZE) {
        let mut query = ModelQuery::new(relation.target_table).where_(Condition::InList {
            expr: Expr::column(relation.target_column.clone()),
            values: chunk.to_vec(),
        });
        if let Some(filter) = relation.filter.clone() {
            query = query.where_(filter);
        }
        for extension in &relation.child_extensions {
            query = query.with_extension_boxed(extension.clone());
        }
        for child in &relation.children {
            query = query.with_boxed(child.clone());
        }
        for aggregate in &relation.child_aggregates {
            query = query.with_aggregate_boxed(aggregate.clone());
        }
        entries.extend(
            query
                .fetch_entries_dyn(executor)
                .await
                .map_err(|error| relation_load_error(relation, "load related models", error))?,
        );
    }

    Ok(entries)
}

fn hydrate_many_to_many_models<From, To, Pivot>(
    relation: &ManyToManyDef<From, To, Pivot>,
    records: &[DbRecord],
) -> Result<Vec<To>>
where
    To: Model,
    Pivot: 'static,
{
    records
        .iter()
        .map(|record| relation.target_table.hydrate_record(record))
        .collect::<Result<Vec<_>>>()
        .map_err(|error| many_to_many_load_error(relation, "hydrate related models", error))
}

fn relation_parent_key<From, To>(
    relation: &RelationDef<From, To>,
    parent: &From,
) -> Result<Option<DbValue>>
where
    To: 'static,
{
    catch_sync_panic(|| (relation.parent_key)(parent)).map_err(|panic| {
        relation_load_error(
            relation,
            "read parent key",
            relation_callback_panic_error("relation parent key callback", panic),
        )
    })
}

fn many_to_many_parent_key<From, To, Pivot>(
    relation: &ManyToManyDef<From, To, Pivot>,
    parent: &From,
) -> Result<Option<DbValue>>
where
    To: 'static,
    Pivot: 'static,
{
    catch_sync_panic(|| (relation.parent_key)(parent)).map_err(|panic| {
        many_to_many_load_error(
            relation,
            "read parent key",
            relation_callback_panic_error("many-to-many parent key callback", panic),
        )
    })
}

fn relation_is_loaded<From, To>(
    relation: &RelationDef<From, To>,
    is_loaded: &Arc<IsLoadedFn<From>>,
    parent: &From,
) -> Result<bool>
where
    To: 'static,
{
    catch_sync_panic(|| is_loaded(parent)).map_err(|panic| {
        relation_load_error(
            relation,
            "check loaded state",
            relation_callback_panic_error("relation is_loaded callback", panic),
        )
    })
}

fn many_to_many_is_loaded<From, To, Pivot>(
    relation: &ManyToManyDef<From, To, Pivot>,
    is_loaded: &Arc<IsLoadedFn<From>>,
    parent: &From,
) -> Result<bool>
where
    To: 'static,
    Pivot: 'static,
{
    catch_sync_panic(|| is_loaded(parent)).map_err(|panic| {
        many_to_many_load_error(
            relation,
            "check loaded state",
            relation_callback_panic_error("many-to-many is_loaded callback", panic),
        )
    })
}

fn attach_relation_values<From, To>(
    relation: &RelationDef<From, To>,
    parent: &mut From,
    values: Vec<To>,
) -> Result<()>
where
    To: 'static,
{
    let result = match &relation.attach {
        RelationAttach::Many(attach) => catch_sync_panic(|| attach(parent, values)),
        RelationAttach::One(attach) => {
            catch_sync_panic(|| attach(parent, values.into_iter().next()))
        }
    };

    result.map_err(|panic| {
        relation_load_error(
            relation,
            "attach related models",
            relation_callback_panic_error("relation attach callback", panic),
        )
    })
}

fn attach_many_to_many_values<From, To, Pivot>(
    relation: &ManyToManyDef<From, To, Pivot>,
    parent: &mut From,
    children: Vec<To>,
) -> Result<()>
where
    To: 'static,
    Pivot: 'static,
{
    catch_sync_panic(|| (relation.attach)(parent, children)).map_err(|panic| {
        many_to_many_load_error(
            relation,
            "attach related models",
            relation_callback_panic_error("many-to-many attach callback", panic),
        )
    })
}

fn attach_relation_aggregate_value<From, To, Value>(
    relation: &RelationDef<From, To>,
    attach: &Arc<AttachAggregateFn<From, Value>>,
    parent: &mut From,
    value: Value,
) -> Result<()>
where
    To: 'static,
{
    catch_sync_panic(|| attach(parent, value)).map_err(|panic| {
        relation_load_error(
            relation,
            "attach aggregate value",
            relation_callback_panic_error("relation aggregate attach callback", panic),
        )
    })
}

fn attach_many_to_many_aggregate_value<From, To, Pivot, Value>(
    relation: &ManyToManyDef<From, To, Pivot>,
    attach: &Arc<AttachAggregateFn<From, Value>>,
    parent: &mut From,
    value: Value,
) -> Result<()>
where
    To: 'static,
    Pivot: 'static,
{
    catch_sync_panic(|| attach(parent, value)).map_err(|panic| {
        many_to_many_load_error(
            relation,
            "attach aggregate value",
            relation_callback_panic_error("many-to-many aggregate attach callback", panic),
        )
    })
}

fn relation_callback_panic_error(callback: &'static str, panic: Box<dyn Any + Send>) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        target: "foundry.database",
        callback = callback,
        panic = %message,
        "relation callback panicked"
    );
    Error::message(format!("{callback} panicked: {message}"))
}

fn relation_load_error<From, To>(
    relation: &RelationDef<From, To>,
    action: &str,
    error: Error,
) -> Error
where
    To: 'static,
{
    Error::message(format!(
        "relation `{}` failed to {action} from target `{}`: {error}",
        relation.name,
        relation.target_table.name()
    ))
}

fn many_to_many_load_error<From, To, Pivot>(
    relation: &ManyToManyDef<From, To, Pivot>,
    action: &str,
    error: Error,
) -> Error
where
    To: 'static,
    Pivot: 'static,
{
    Error::message(format!(
        "many-to-many relation `{}` failed to {action} from target `{}` via pivot `{}`: {error}",
        relation.name,
        relation.target_table.name(),
        relation.pivot_table.name
    ))
}

fn group_relation_entries<To>(
    entries: Vec<(DbRecord, To)>,
    target_column: &ColumnRef,
) -> Result<HashMap<String, Vec<To>>> {
    let mut grouped: HashMap<String, Vec<To>> = HashMap::new();
    for (record, model) in entries {
        let key = record
            .get(&target_column.name)
            .ok_or_else(|| {
                Error::message(format!(
                    "missing target relation key `{}` in eager-loaded record",
                    target_column.name
                ))
            })?
            .relation_key();
        grouped.entry(key).or_default().push(model);
    }
    Ok(grouped)
}

async fn fetch_many_to_many_records<From, To, Pivot>(
    executor: &dyn QueryExecutor,
    relation: &ManyToManyDef<From, To, Pivot>,
    keys: Vec<DbValue>,
) -> Result<Vec<DbRecord>>
where
    From: Model,
    To: Model,
    Pivot: Clone + Send + Sync + 'static,
{
    let mut records = Vec::new();
    for chunk in keys.chunks(RELATION_KEY_CHUNK_SIZE) {
        let mut select = SelectNode::from(relation.target_table.table_ref());
        select.columns = relation.target_table.all_select_items();
        select.columns.push(
            SelectItem::new(Expr::column(relation.pivot_parent_column.clone()))
                .aliased(RELATION_GROUP_KEY_ALIAS),
        );
        if let Some(pivot_attacher) = &relation.pivot_attacher {
            select
                .columns
                .extend(pivot_attacher.select_items(&relation.pivot_table.name)?);
        }
        select.joins.push(JoinNode {
            kind: JoinKind::Inner,
            table: relation.pivot_table.clone().into(),
            lateral: false,
            on: Some(Condition::compare(
                Expr::column(relation.target_column.clone()),
                ComparisonOp::Eq,
                Expr::column(relation.pivot_target_column.clone()),
            )),
        });
        let condition = Condition::InList {
            expr: Expr::column(relation.pivot_parent_column.clone()),
            values: chunk.to_vec(),
        };
        select.condition = Some(match relation.filter.clone() {
            Some(filter) => Condition::and([filter, condition]),
            None => condition,
        });

        let compiled = PostgresCompiler::compile(&QueryAst::select(select))?;
        records.extend(executor.query_records(&compiled).await?);
    }
    Ok(records)
}

fn count_parent_relation_keys<From>(
    parents: &[From],
    parent_key: impl Fn(&From) -> Result<Option<DbValue>>,
) -> Result<HashMap<String, usize>> {
    let mut counts = HashMap::new();
    for parent in parents {
        if let Some(key) = parent_key(parent)? {
            *counts.entry(key.relation_key()).or_insert(0) += 1;
        }
    }
    Ok(counts)
}

fn count_parent_relation_keys_for_indices<From>(
    parents: &[From],
    parent_key: impl Fn(&From) -> Result<Option<DbValue>>,
    indices: &[usize],
) -> Result<HashMap<String, usize>> {
    let mut counts = HashMap::new();
    for &index in indices {
        if let Some(key) = parent_key(&parents[index])? {
            *counts.entry(key.relation_key()).or_insert(0) += 1;
        }
    }
    Ok(counts)
}

fn take_grouped_values<To: Clone>(
    grouped: &mut HashMap<String, Vec<To>>,
    remaining: &mut HashMap<String, usize>,
    key: &str,
) -> Vec<To> {
    match remaining.get_mut(key) {
        Some(count) if *count > 1 => {
            *count -= 1;
            grouped.get(key).cloned().unwrap_or_default()
        }
        Some(_) => {
            remaining.remove(key);
            grouped.remove(key).unwrap_or_default()
        }
        None => Vec::new(),
    }
}

fn collect_relation_keys<From>(
    parents: &[From],
    parent_key: impl Fn(&From) -> Result<Option<DbValue>>,
) -> Result<Vec<DbValue>> {
    let mut keys = Vec::new();
    let mut seen = BTreeSet::new();

    for parent in parents {
        if let Some(key) = parent_key(parent)? {
            if seen.insert(key.relation_key()) {
                keys.push(key);
            }
        }
    }

    Ok(keys)
}

async fn execute_relation_aggregate_query<From, To>(
    executor: &dyn QueryExecutor,
    relation: &RelationDef<From, To>,
    kind: AggregateKind,
    keys: Vec<DbValue>,
) -> Result<HashMap<String, DbRecord>>
where
    From: Model,
    To: Model,
{
    execute_grouped_aggregate_query(
        executor,
        relation.target_table.table_ref(),
        relation.filter.clone(),
        relation.target_column.clone(),
        None,
        keys,
        kind,
    )
    .await
}

async fn execute_many_to_many_aggregate_query<From, To, Pivot>(
    executor: &dyn QueryExecutor,
    relation: &ManyToManyDef<From, To, Pivot>,
    kind: AggregateKind,
    keys: Vec<DbValue>,
) -> Result<HashMap<String, DbRecord>>
where
    From: Model,
    To: Model,
    Pivot: Clone + Send + Sync + 'static,
{
    execute_grouped_aggregate_query(
        executor,
        relation.target_table.table_ref(),
        relation.filter.clone(),
        relation.pivot_parent_column.clone(),
        Some(JoinNode {
            kind: JoinKind::Inner,
            table: relation.pivot_table.clone().into(),
            lateral: false,
            on: Some(Condition::compare(
                Expr::column(relation.target_column.clone()),
                ComparisonOp::Eq,
                Expr::column(relation.pivot_target_column.clone()),
            )),
        }),
        keys,
        kind,
    )
    .await
}

async fn execute_grouped_aggregate_query(
    executor: &dyn QueryExecutor,
    from: TableRef,
    filter: Option<Condition>,
    group_key_column: ColumnRef,
    join: Option<JoinNode>,
    keys: Vec<DbValue>,
    kind: AggregateKind,
) -> Result<HashMap<String, DbRecord>> {
    if keys.is_empty() {
        return Ok(HashMap::new());
    }

    let mut grouped = HashMap::new();
    for chunk in keys.chunks(RELATION_KEY_CHUNK_SIZE) {
        let mut select = SelectNode::from(from.clone());
        select.columns = vec![SelectItem::new(Expr::column(group_key_column.clone()))
            .aliased(RELATION_GROUP_KEY_ALIAS)];
        if let Some(join) = join.clone() {
            select.joins.push(join);
        }
        let condition = Condition::InList {
            expr: Expr::column(group_key_column.clone()),
            values: chunk.to_vec(),
        };
        select.condition = Some(match filter.clone() {
            Some(filter) => Condition::and([filter, condition]),
            None => condition,
        });
        select.group_by.push(Expr::column(group_key_column.clone()));
        select.aggregates.push(kind.clone().node());

        let compiled = PostgresCompiler::compile(&QueryAst::select(select))?;
        let rows = executor.query_records(&compiled).await?;
        for row in rows {
            let key = row
                .get(RELATION_GROUP_KEY_ALIAS)
                .ok_or_else(|| Error::message("missing relation aggregate group key"))?
                .relation_key();
            grouped.insert(key, row);
        }
    }
    Ok(grouped)
}

fn merge_condition(existing: Option<Condition>, next: Condition) -> Option<Condition> {
    Some(match existing {
        Some(existing) => Condition::and([existing, next]),
        None => next,
    })
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::{
        belongs_to, has_many, infer_collection_relation_name, infer_singular_relation_name,
        many_to_many, RelationLoader, PIVOT_ALIAS_PREFIX, RELATION_AGGREGATE_ALIAS,
        RELATION_GROUP_KEY_ALIAS,
    };
    use crate::database::{
        DbRecord, DbValue, FromDbValue, Loaded, Projection, QueryExecutionOptions, QueryExecutor,
    };
    use crate::foundation::{Error, Result};

    #[derive(Clone)]
    struct StaticQueryExecutor {
        records: Vec<DbRecord>,
    }

    #[async_trait]
    impl QueryExecutor for StaticQueryExecutor {
        async fn raw_query_with(
            &self,
            _sql: &str,
            _bindings: &[DbValue],
            _options: QueryExecutionOptions,
        ) -> Result<Vec<DbRecord>> {
            Ok(self.records.clone())
        }

        async fn raw_execute_with(
            &self,
            _sql: &str,
            _bindings: &[DbValue],
            _options: QueryExecutionOptions,
        ) -> Result<u64> {
            Ok(0)
        }
    }

    #[derive(Clone, Debug, PartialEq)]
    struct RelationPanicText(String);

    impl FromDbValue for RelationPanicText {
        fn from_db_value(value: &DbValue) -> Result<Self> {
            match value {
                DbValue::Text(value) if value == "panic-child" => {
                    panic!("relation child hydrate boom")
                }
                DbValue::Text(value) if value == "panic-pivot" => {
                    panic!("relation pivot hydrate boom")
                }
                DbValue::Text(value) => Ok(Self(value.clone())),
                _ => Err(Error::message("expected text value")),
            }
        }
    }

    #[derive(Debug, PartialEq, crate::Model)]
    #[foundry(table = "relation_context_parents", primary_key_strategy = "manual")]
    struct RelationContextParent {
        id: i64,
        children: Loaded<Vec<RelationContextChild>>,
        tags: Loaded<Vec<RelationContextTag>>,
    }

    #[derive(Debug, PartialEq, crate::Model)]
    #[foundry(table = "relation_context_children", primary_key_strategy = "manual")]
    struct RelationContextChild {
        id: i64,
        parent_id: i64,
        #[foundry(db_type = "text")]
        value: RelationPanicText,
    }

    #[derive(Debug, PartialEq, crate::Model)]
    #[foundry(table = "relation_context_tags", primary_key_strategy = "manual")]
    struct RelationContextTag {
        id: i64,
        name: String,
        pivot: Loaded<RelationContextPivot>,
    }

    #[derive(Debug, PartialEq, crate::Model)]
    #[foundry(table = "nullable_relation_parents", primary_key_strategy = "manual")]
    struct NullableRelationParent {
        id: i64,
    }

    #[derive(Debug, PartialEq, crate::Model)]
    #[foundry(table = "nullable_relation_children", primary_key_strategy = "manual")]
    struct NullableRelationChild {
        id: i64,
        parent_id: Option<i64>,
        parent: Loaded<Option<NullableRelationParent>>,
    }

    #[derive(Clone, Debug, PartialEq, crate::Projection)]
    struct RelationContextPivot {
        #[foundry(db_type = "text")]
        role: RelationPanicText,
    }

    #[test]
    fn infers_collection_relation_names_from_table_names() {
        assert_eq!(infer_collection_relation_name("merchants"), "merchants");
        assert_eq!(
            infer_collection_relation_name("public.order_items"),
            "order_items"
        );
    }

    #[test]
    fn infers_singular_relation_names_from_plural_table_names() {
        assert_eq!(infer_singular_relation_name("countries"), "country");
        assert_eq!(
            infer_singular_relation_name("public.categories"),
            "category"
        );
        assert_eq!(infer_singular_relation_name("products"), "product");
    }

    #[test]
    fn belongs_to_accepts_nullable_foreign_key_columns() {
        let relation = belongs_to(
            NullableRelationChild::PARENT_ID,
            NullableRelationParent::ID,
            |child: &NullableRelationChild| child.parent_id,
            |child, parent| child.parent = Loaded::new(parent),
        );

        assert_eq!(
            relation.node().kind,
            crate::database::RelationKind::BelongsTo
        );
    }

    #[tokio::test]
    async fn relation_hydration_errors_include_relation_context() {
        let mut child_record = DbRecord::new();
        child_record.insert("id", DbValue::from(2_i64));
        child_record.insert("parent_id", DbValue::from(1_i64));
        child_record.insert("value", DbValue::from("panic-child"));
        let executor = StaticQueryExecutor {
            records: vec![child_record],
        };
        let relation = has_many(
            RelationContextParent::ID,
            RelationContextChild::PARENT_ID,
            |parent: &RelationContextParent| parent.id,
            |parent, children| parent.children = Loaded::new(children),
        )
        .named("panic_children");
        let mut parents = vec![RelationContextParent {
            id: 1,
            children: Loaded::Unloaded,
            tags: Loaded::Unloaded,
        }];

        let error = relation.load(&executor, &mut parents).await.unwrap_err();
        let message = error.to_string();

        assert!(message.contains(
            "relation `panic_children` failed to load related models from target `relation_context_children`"
        ));
        assert!(message.contains("model query failed to hydrate root rows"));
        assert!(message.contains("hydration panicked: relation child hydrate boom"));
    }

    #[tokio::test]
    async fn many_to_many_pivot_hydration_errors_include_relation_context() {
        let mut tag_record = DbRecord::new();
        tag_record.insert("id", DbValue::from(7_i64));
        tag_record.insert("name", DbValue::from("security"));
        tag_record.insert(RELATION_GROUP_KEY_ALIAS, DbValue::from(1_i64));
        tag_record.insert(
            format!("{PIVOT_ALIAS_PREFIX}role"),
            DbValue::from("panic-pivot"),
        );
        let executor = StaticQueryExecutor {
            records: vec![tag_record],
        };
        let relation = many_to_many::<RelationContextParent, RelationContextTag, (), i64, i64>(
            RelationContextParent::ID,
            "relation_context_tag_links",
            "parent_id",
            "tag_id",
            RelationContextTag::ID,
            |parent: &RelationContextParent| parent.id,
            |parent, tags| parent.tags = Loaded::new(tags),
        )
        .named("panic_tags")
        .with_pivot(RelationContextPivot::projection_meta(), |tag, pivot| {
            tag.pivot = Loaded::new(pivot);
        });
        let mut parents = vec![RelationContextParent {
            id: 1,
            children: Loaded::Unloaded,
            tags: Loaded::Unloaded,
        }];

        let error = relation.load(&executor, &mut parents).await.unwrap_err();
        let message = error.to_string();

        assert!(message.contains(
            "many-to-many relation `panic_tags` failed to hydrate pivot projection from target `relation_context_tags` via pivot `relation_context_tag_links`"
        ));
        assert!(message.contains("projection `"));
        assert!(message.contains("hydration panicked: relation pivot hydrate boom"));
    }

    #[tokio::test]
    async fn relation_attach_panics_include_relation_context() {
        let mut child_record = DbRecord::new();
        child_record.insert("id", DbValue::from(2_i64));
        child_record.insert("parent_id", DbValue::from(1_i64));
        child_record.insert("value", DbValue::from("ok"));
        let executor = StaticQueryExecutor {
            records: vec![child_record],
        };
        let relation = has_many(
            RelationContextParent::ID,
            RelationContextChild::PARENT_ID,
            |parent: &RelationContextParent| parent.id,
            |_parent, _children| panic!("relation attach boom"),
        )
        .named("panic_attach_children");
        let mut parents = vec![RelationContextParent {
            id: 1,
            children: Loaded::Unloaded,
            tags: Loaded::Unloaded,
        }];

        let error = relation.load(&executor, &mut parents).await.unwrap_err();
        let message = error.to_string();

        assert!(message.contains(
            "relation `panic_attach_children` failed to attach related models from target `relation_context_children`"
        ));
        assert!(message.contains("relation attach callback panicked: relation attach boom"));
    }

    #[tokio::test]
    async fn many_to_many_attach_panics_include_relation_context() {
        let mut tag_record = DbRecord::new();
        tag_record.insert("id", DbValue::from(7_i64));
        tag_record.insert("name", DbValue::from("security"));
        tag_record.insert(RELATION_GROUP_KEY_ALIAS, DbValue::from(1_i64));
        let executor = StaticQueryExecutor {
            records: vec![tag_record],
        };
        let relation = many_to_many::<RelationContextParent, RelationContextTag, (), i64, i64>(
            RelationContextParent::ID,
            "relation_context_tag_links",
            "parent_id",
            "tag_id",
            RelationContextTag::ID,
            |parent: &RelationContextParent| parent.id,
            |_parent, _tags| panic!("many attach boom"),
        )
        .named("panic_attach_tags");
        let mut parents = vec![RelationContextParent {
            id: 1,
            children: Loaded::Unloaded,
            tags: Loaded::Unloaded,
        }];

        let error = relation.load(&executor, &mut parents).await.unwrap_err();
        let message = error.to_string();

        assert!(message.contains(
            "many-to-many relation `panic_attach_tags` failed to attach related models from target `relation_context_tags` via pivot `relation_context_tag_links`"
        ));
        assert!(message.contains("many-to-many attach callback panicked: many attach boom"));
    }

    #[tokio::test]
    async fn pivot_attach_panics_include_relation_context() {
        let mut tag_record = DbRecord::new();
        tag_record.insert("id", DbValue::from(7_i64));
        tag_record.insert("name", DbValue::from("security"));
        tag_record.insert(RELATION_GROUP_KEY_ALIAS, DbValue::from(1_i64));
        tag_record.insert(format!("{PIVOT_ALIAS_PREFIX}role"), DbValue::from("ok"));
        let executor = StaticQueryExecutor {
            records: vec![tag_record],
        };
        let relation = many_to_many::<RelationContextParent, RelationContextTag, (), i64, i64>(
            RelationContextParent::ID,
            "relation_context_tag_links",
            "parent_id",
            "tag_id",
            RelationContextTag::ID,
            |parent: &RelationContextParent| parent.id,
            |parent, tags| parent.tags = Loaded::new(tags),
        )
        .named("panic_pivot_attach_tags")
        .with_pivot(RelationContextPivot::projection_meta(), |_tag, _pivot| {
            panic!("pivot attach boom");
        });
        let mut parents = vec![RelationContextParent {
            id: 1,
            children: Loaded::Unloaded,
            tags: Loaded::Unloaded,
        }];

        let error = relation.load(&executor, &mut parents).await.unwrap_err();
        let message = error.to_string();

        assert!(message.contains(
            "many-to-many relation `panic_pivot_attach_tags` failed to hydrate pivot projection from target `relation_context_tags` via pivot `relation_context_tag_links`"
        ));
        assert!(message.contains("pivot attach callback panicked: pivot attach boom"));
    }

    #[tokio::test]
    async fn aggregate_attach_panics_include_relation_context() {
        let mut aggregate_record = DbRecord::new();
        aggregate_record.insert(RELATION_GROUP_KEY_ALIAS, DbValue::from(1_i64));
        aggregate_record.insert(RELATION_AGGREGATE_ALIAS, DbValue::from(2_i64));
        let executor = StaticQueryExecutor {
            records: vec![aggregate_record],
        };
        let aggregate = has_many(
            RelationContextParent::ID,
            RelationContextChild::PARENT_ID,
            |parent: &RelationContextParent| parent.id,
            |parent, children| parent.children = Loaded::new(children),
        )
        .named("panic_child_count")
        .count(|_parent, _count| panic!("aggregate attach boom"))
        .into_loader();
        let mut parents = vec![RelationContextParent {
            id: 1,
            children: Loaded::Unloaded,
            tags: Loaded::Unloaded,
        }];

        let error = aggregate.load(&executor, &mut parents).await.unwrap_err();
        let message = error.to_string();

        assert!(message.contains(
            "relation `panic_child_count` failed to attach aggregate value from target `relation_context_children`"
        ));
        assert!(
            message.contains("relation aggregate attach callback panicked: aggregate attach boom")
        );
    }

    #[tokio::test]
    async fn many_to_many_aggregate_attach_panics_include_relation_context() {
        let mut aggregate_record = DbRecord::new();
        aggregate_record.insert(RELATION_GROUP_KEY_ALIAS, DbValue::from(1_i64));
        aggregate_record.insert(RELATION_AGGREGATE_ALIAS, DbValue::from(2_i64));
        let executor = StaticQueryExecutor {
            records: vec![aggregate_record],
        };
        let aggregate = many_to_many::<RelationContextParent, RelationContextTag, (), i64, i64>(
            RelationContextParent::ID,
            "relation_context_tag_links",
            "parent_id",
            "tag_id",
            RelationContextTag::ID,
            |parent: &RelationContextParent| parent.id,
            |parent, tags| parent.tags = Loaded::new(tags),
        )
        .named("panic_tag_count")
        .count(|_parent, _count| panic!("many aggregate attach boom"))
        .into_loader();
        let mut parents = vec![RelationContextParent {
            id: 1,
            children: Loaded::Unloaded,
            tags: Loaded::Unloaded,
        }];

        let error = aggregate.load(&executor, &mut parents).await.unwrap_err();
        let message = error.to_string();

        assert!(message.contains(
            "many-to-many relation `panic_tag_count` failed to attach aggregate value from target `relation_context_tags` via pivot `relation_context_tag_links`"
        ));
        assert!(message.contains(
            "many-to-many aggregate attach callback panicked: many aggregate attach boom"
        ));
    }

    #[tokio::test]
    async fn relation_parent_key_panics_include_relation_context() {
        let executor = StaticQueryExecutor {
            records: Vec::new(),
        };
        let relation = has_many(
            RelationContextParent::ID,
            RelationContextChild::PARENT_ID,
            |_parent: &RelationContextParent| -> i64 { panic!("relation parent key boom") },
            |parent, children| parent.children = Loaded::new(children),
        )
        .named("panic_parent_key_children");
        let mut parents = vec![RelationContextParent {
            id: 1,
            children: Loaded::Unloaded,
            tags: Loaded::Unloaded,
        }];

        let error = relation.load(&executor, &mut parents).await.unwrap_err();
        let message = error.to_string();

        assert!(message.contains(
            "relation `panic_parent_key_children` failed to read parent key from target `relation_context_children`"
        ));
        assert!(message.contains("relation parent key callback panicked: relation parent key boom"));
    }

    #[tokio::test]
    async fn relation_is_loaded_panics_include_relation_context() {
        let executor = StaticQueryExecutor {
            records: Vec::new(),
        };
        let relation = has_many(
            RelationContextParent::ID,
            RelationContextChild::PARENT_ID,
            |parent: &RelationContextParent| parent.id,
            |parent, children| parent.children = Loaded::new(children),
        )
        .named("panic_loaded_children")
        .is_loaded(|_parent| panic!("relation is loaded boom"));
        let mut parents = vec![RelationContextParent {
            id: 1,
            children: Loaded::Unloaded,
            tags: Loaded::Unloaded,
        }];

        let error = relation
            .load_missing(&executor, &mut parents)
            .await
            .unwrap_err();
        let message = error.to_string();

        assert!(message.contains(
            "relation `panic_loaded_children` failed to check loaded state from target `relation_context_children`"
        ));
        assert!(message.contains("relation is_loaded callback panicked: relation is loaded boom"));
    }

    #[tokio::test]
    async fn many_to_many_parent_key_panics_include_relation_context() {
        let executor = StaticQueryExecutor {
            records: Vec::new(),
        };
        let relation = many_to_many::<RelationContextParent, RelationContextTag, (), i64, i64>(
            RelationContextParent::ID,
            "relation_context_tag_links",
            "parent_id",
            "tag_id",
            RelationContextTag::ID,
            |_parent: &RelationContextParent| -> i64 { panic!("many parent key boom") },
            |parent, tags| parent.tags = Loaded::new(tags),
        )
        .named("panic_parent_key_tags");
        let mut parents = vec![RelationContextParent {
            id: 1,
            children: Loaded::Unloaded,
            tags: Loaded::Unloaded,
        }];

        let error = relation.load(&executor, &mut parents).await.unwrap_err();
        let message = error.to_string();

        assert!(message.contains(
            "many-to-many relation `panic_parent_key_tags` failed to read parent key from target `relation_context_tags` via pivot `relation_context_tag_links`"
        ));
        assert!(message.contains("many-to-many parent key callback panicked: many parent key boom"));
    }

    #[tokio::test]
    async fn many_to_many_is_loaded_panics_include_relation_context() {
        let executor = StaticQueryExecutor {
            records: Vec::new(),
        };
        let relation = many_to_many::<RelationContextParent, RelationContextTag, (), i64, i64>(
            RelationContextParent::ID,
            "relation_context_tag_links",
            "parent_id",
            "tag_id",
            RelationContextTag::ID,
            |parent: &RelationContextParent| parent.id,
            |parent, tags| parent.tags = Loaded::new(tags),
        )
        .named("panic_loaded_tags")
        .is_loaded(|_parent| panic!("many is loaded boom"));
        let mut parents = vec![RelationContextParent {
            id: 1,
            children: Loaded::Unloaded,
            tags: Loaded::Unloaded,
        }];

        let error = relation
            .load_missing(&executor, &mut parents)
            .await
            .unwrap_err();
        let message = error.to_string();

        assert!(message.contains(
            "many-to-many relation `panic_loaded_tags` failed to check loaded state from target `relation_context_tags` via pivot `relation_context_tag_links`"
        ));
        assert!(message.contains("many-to-many is_loaded callback panicked: many is loaded boom"));
    }

    #[tokio::test]
    async fn aggregate_parent_key_panics_include_relation_context() {
        let executor = StaticQueryExecutor {
            records: Vec::new(),
        };
        let aggregate = has_many(
            RelationContextParent::ID,
            RelationContextChild::PARENT_ID,
            |_parent: &RelationContextParent| -> i64 { panic!("aggregate parent key boom") },
            |parent, children| parent.children = Loaded::new(children),
        )
        .named("panic_child_count_key")
        .count(|_parent, _count| {})
        .into_loader();
        let mut parents = vec![RelationContextParent {
            id: 1,
            children: Loaded::Unloaded,
            tags: Loaded::Unloaded,
        }];

        let error = aggregate.load(&executor, &mut parents).await.unwrap_err();
        let message = error.to_string();

        assert!(message.contains(
            "relation `panic_child_count_key` failed to read parent key from target `relation_context_children`"
        ));
        assert!(
            message.contains("relation parent key callback panicked: aggregate parent key boom")
        );
    }

    #[tokio::test]
    async fn many_to_many_aggregate_parent_key_panics_include_relation_context() {
        let executor = StaticQueryExecutor {
            records: Vec::new(),
        };
        let aggregate = many_to_many::<RelationContextParent, RelationContextTag, (), i64, i64>(
            RelationContextParent::ID,
            "relation_context_tag_links",
            "parent_id",
            "tag_id",
            RelationContextTag::ID,
            |_parent: &RelationContextParent| -> i64 { panic!("many aggregate parent key boom") },
            |parent, tags| parent.tags = Loaded::new(tags),
        )
        .named("panic_tag_count_key")
        .count(|_parent, _count| {})
        .into_loader();
        let mut parents = vec![RelationContextParent {
            id: 1,
            children: Loaded::Unloaded,
            tags: Loaded::Unloaded,
        }];

        let error = aggregate.load(&executor, &mut parents).await.unwrap_err();
        let message = error.to_string();

        assert!(message.contains(
            "many-to-many relation `panic_tag_count_key` failed to read parent key from target `relation_context_tags` via pivot `relation_context_tag_links`"
        ));
        assert!(message
            .contains("many-to-many parent key callback panicked: many aggregate parent key boom"));
    }
}
