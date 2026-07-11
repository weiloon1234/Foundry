use serde::{Deserialize, Serialize};

use crate::auth::Actor;
use crate::database::{
    DbRecord, DbValue, Expr, OrderBy, Paginated, Pagination, Query, QueryExecutor, Sql,
};
use crate::foundation::{AppContext, Error, Result};
use crate::support::{Collection, DateTime, ModelId};

use super::{callback, Notifiable, NOTIFICATIONS_TABLE};

/// A database-backed notification visible to one notifiable scope.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DatabaseNotification {
    pub id: ModelId<DatabaseNotification>,
    pub notifiable_type: String,
    pub notifiable_id: String,
    pub notification_type: String,
    pub data: serde_json::Value,
    pub read_at: Option<DateTime>,
    pub created_at: DateTime,
}

impl DatabaseNotification {
    pub fn is_read(&self) -> bool {
        self.read_at.is_some()
    }

    pub fn is_unread(&self) -> bool {
        self.read_at.is_none()
    }
}

/// Validated database-notification ownership scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatabaseNotificationScope {
    notifiable_type: String,
    notifiable_id: String,
}

impl DatabaseNotificationScope {
    pub fn new(
        notifiable_type: impl Into<String>,
        notifiable_id: impl Into<String>,
    ) -> Result<Self> {
        let notifiable_type = notifiable_type.into();
        let notifiable_id = notifiable_id.into();
        validate_scope_value("type", &notifiable_type)?;
        validate_scope_value("id", &notifiable_id)?;
        Ok(Self {
            notifiable_type,
            notifiable_id,
        })
    }

    pub fn for_notifiable(notifiable: &dyn Notifiable) -> Result<Self> {
        Self::new(
            callback::notifiable_type(notifiable)?,
            callback::notifiable_id(notifiable)?,
        )
    }

    /// Scope notifications to an authenticated actor using its guard as the
    /// notifiable type and its actor ID as the notifiable ID.
    pub fn for_actor(actor: &Actor) -> Result<Self> {
        Self::new(actor.guard.to_string(), actor.id.clone())
    }

    /// Scope an actor to an explicit notifiable type when the application's
    /// stored morph type differs from the authentication guard ID.
    pub fn for_actor_as(actor: &Actor, notifiable_type: impl Into<String>) -> Result<Self> {
        Self::new(notifiable_type, actor.id.clone())
    }

    pub fn notifiable_type(&self) -> &str {
        &self.notifiable_type
    }

    pub fn notifiable_id(&self) -> &str {
        &self.notifiable_id
    }
}

/// Scoped repository for reading and mutating database notifications.
///
/// Every operation applies both `notifiable_type` and `notifiable_id`; callers
/// cannot use this repository to issue an unscoped notification query.
#[derive(Clone, Debug)]
pub struct DatabaseNotificationRepository {
    scope: DatabaseNotificationScope,
}

impl DatabaseNotificationRepository {
    pub fn new(
        notifiable_type: impl Into<String>,
        notifiable_id: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self::from_scope(DatabaseNotificationScope::new(
            notifiable_type,
            notifiable_id,
        )?))
    }

    pub fn from_scope(scope: DatabaseNotificationScope) -> Self {
        Self { scope }
    }

    pub fn for_notifiable(notifiable: &dyn Notifiable) -> Result<Self> {
        DatabaseNotificationScope::for_notifiable(notifiable).map(Self::from_scope)
    }

    pub fn for_actor(actor: &Actor) -> Result<Self> {
        DatabaseNotificationScope::for_actor(actor).map(Self::from_scope)
    }

    pub fn for_actor_as(actor: &Actor, notifiable_type: impl Into<String>) -> Result<Self> {
        DatabaseNotificationScope::for_actor_as(actor, notifiable_type).map(Self::from_scope)
    }

    pub fn scope(&self) -> &DatabaseNotificationScope {
        &self.scope
    }

    pub async fn list(&self, app: &AppContext) -> Result<Vec<DatabaseNotification>> {
        self.list_with(app).await
    }

    pub async fn list_with<E>(&self, executor: &E) -> Result<Vec<DatabaseNotification>>
    where
        E: QueryExecutor + ?Sized,
    {
        self.fetch_with(executor, ReadFilter::Any).await
    }

    pub async fn paginate(
        &self,
        app: &AppContext,
        pagination: Pagination,
    ) -> Result<Paginated<DatabaseNotification>> {
        self.paginate_with(app, pagination).await
    }

    pub async fn paginate_with<E>(
        &self,
        executor: &E,
        pagination: Pagination,
    ) -> Result<Paginated<DatabaseNotification>>
    where
        E: QueryExecutor + ?Sized,
    {
        let page = self
            .query(ReadFilter::Any)
            .paginate(executor, pagination)
            .await?;
        let data = page
            .data
            .into_iter()
            .map(|record| hydrate_database_notification(&record))
            .collect::<Result<Collection<_>>>()?;
        Ok(Paginated {
            data,
            pagination: page.pagination,
            total: page.total,
        })
    }

    /// List unread notifications in deterministic newest-first order.
    pub async fn unread(&self, app: &AppContext) -> Result<Vec<DatabaseNotification>> {
        self.unread_with(app).await
    }

    pub async fn unread_with<E>(&self, executor: &E) -> Result<Vec<DatabaseNotification>>
    where
        E: QueryExecutor + ?Sized,
    {
        self.fetch_with(executor, ReadFilter::Unread).await
    }

    /// List read notifications in deterministic newest-first order.
    pub async fn read(&self, app: &AppContext) -> Result<Vec<DatabaseNotification>> {
        self.read_with(app).await
    }

    pub async fn read_with<E>(&self, executor: &E) -> Result<Vec<DatabaseNotification>>
    where
        E: QueryExecutor + ?Sized,
    {
        self.fetch_with(executor, ReadFilter::Read).await
    }

    pub async fn unread_count(&self, app: &AppContext) -> Result<u64> {
        self.unread_count_with(app).await
    }

    pub async fn unread_count_with<E>(&self, executor: &E) -> Result<u64>
    where
        E: QueryExecutor + ?Sized,
    {
        self.query(ReadFilter::Unread).count(executor).await
    }

    pub async fn mark_read(
        &self,
        app: &AppContext,
        id: ModelId<DatabaseNotification>,
    ) -> Result<bool> {
        self.mark_read_with(app, id).await
    }

    pub async fn mark_read_with<E>(
        &self,
        executor: &E,
        id: ModelId<DatabaseNotification>,
    ) -> Result<bool>
    where
        E: QueryExecutor + ?Sized,
    {
        let affected = self
            .scope_query(Query::update_table(NOTIFICATIONS_TABLE))
            .set_expr("read_at", Sql::now())
            .where_eq("id", DbValue::Uuid(id.into_uuid()))
            .where_(Expr::column("read_at").is_null())
            .execute(executor)
            .await?;
        Ok(affected > 0)
    }

    /// Mark every currently unread notification in this scope as read.
    pub async fn mark_all_read(&self, app: &AppContext) -> Result<u64> {
        self.mark_all_read_with(app).await
    }

    pub async fn mark_all_read_with<E>(&self, executor: &E) -> Result<u64>
    where
        E: QueryExecutor + ?Sized,
    {
        self.scope_query(Query::update_table(NOTIFICATIONS_TABLE))
            .set_expr("read_at", Sql::now())
            .where_(Expr::column("read_at").is_null())
            .execute(executor)
            .await
    }

    pub async fn delete(
        &self,
        app: &AppContext,
        id: ModelId<DatabaseNotification>,
    ) -> Result<bool> {
        self.delete_with(app, id).await
    }

    pub async fn delete_with<E>(
        &self,
        executor: &E,
        id: ModelId<DatabaseNotification>,
    ) -> Result<bool>
    where
        E: QueryExecutor + ?Sized,
    {
        let affected = self
            .scope_query(Query::delete_from(NOTIFICATIONS_TABLE))
            .where_eq("id", DbValue::Uuid(id.into_uuid()))
            .execute(executor)
            .await?;
        Ok(affected > 0)
    }

    async fn fetch_with<E>(
        &self,
        executor: &E,
        filter: ReadFilter,
    ) -> Result<Vec<DatabaseNotification>>
    where
        E: QueryExecutor + ?Sized,
    {
        self.query(filter)
            .get(executor)
            .await?
            .iter()
            .map(hydrate_database_notification)
            .collect()
    }

    fn query(&self, filter: ReadFilter) -> Query {
        let query = self.scope_query(Query::table(NOTIFICATIONS_TABLE).select([
            "id",
            "notifiable_type",
            "notifiable_id",
            "type",
            "data",
            "read_at",
            "created_at",
        ]));
        let query = match filter {
            ReadFilter::Any => query,
            ReadFilter::Unread => query.where_(Expr::column("read_at").is_null()),
            ReadFilter::Read => query.where_(Expr::column("read_at").is_not_null()),
        };
        query
            .order_by(OrderBy::desc("created_at"))
            .order_by(OrderBy::desc("id"))
    }

    fn scope_query(&self, query: Query) -> Query {
        query
            .where_eq(
                "notifiable_type",
                DbValue::Text(self.scope.notifiable_type.clone()),
            )
            .where_eq(
                "notifiable_id",
                DbValue::Text(self.scope.notifiable_id.clone()),
            )
    }
}

#[derive(Clone, Copy)]
enum ReadFilter {
    Any,
    Unread,
    Read,
}

fn validate_scope_value(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(Error::message(format!(
            "database notification notifiable {name} cannot be empty"
        )));
    }
    Ok(())
}

fn hydrate_database_notification(record: &DbRecord) -> Result<DatabaseNotification> {
    Ok(DatabaseNotification {
        id: record.decode("id")?,
        notifiable_type: record.try_text("notifiable_type")?,
        notifiable_id: record.try_text("notifiable_id")?,
        notification_type: record.try_text("type")?,
        data: record.decode("data")?,
        read_at: record.decode("read_at")?,
        created_at: record.decode("created_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::GuardId;

    struct TeamMember;

    impl Notifiable for TeamMember {
        fn notification_id(&self) -> String {
            "member-1".to_string()
        }

        fn notifiable_type(&self) -> &str {
            "team_member"
        }
    }

    #[test]
    fn scopes_are_validated_and_derived_from_notifiables_and_actors() {
        for (kind, id) in [("", "id"), ("kind", " ")] {
            assert!(DatabaseNotificationScope::new(kind, id).is_err());
        }

        let notifiable = DatabaseNotificationScope::for_notifiable(&TeamMember).unwrap();
        assert_eq!(notifiable.notifiable_type(), "team_member");
        assert_eq!(notifiable.notifiable_id(), "member-1");

        let actor = Actor::new("member-1", GuardId::new("api"));
        let guarded = DatabaseNotificationScope::for_actor(&actor).unwrap();
        assert_eq!(guarded.notifiable_type(), "api");
        let explicit = DatabaseNotificationScope::for_actor_as(&actor, "team_member").unwrap();
        assert_eq!(explicit, notifiable);
    }
}
