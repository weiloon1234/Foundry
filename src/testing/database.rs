use async_trait::async_trait;

use crate::database::{
    AfterCommitCallback, AfterCommitSink, DatabaseTransaction, DbRecord, Model, ModelQuery,
    ModelWriteExecutor, QueryExecutionOptions, QueryExecutor,
};
use crate::foundation::{AppContext, AppTransaction, Error, Result};

/// Application transaction intended to be rolled back at the end of a database test.
///
/// It implements the same query/model-write traits as [`AppTransaction`], so model
/// factories, lifecycle hooks, and database assertions all share the isolated transaction.
pub struct DatabaseTestTransaction {
    transaction: Option<AppTransaction>,
}

impl DatabaseTestTransaction {
    pub async fn begin(app: &AppContext) -> Result<Self> {
        Ok(Self {
            transaction: Some(app.begin_transaction().await?),
        })
    }

    pub fn app(&self) -> &AppContext {
        self.inner().app()
    }

    pub fn transaction(&self) -> &DatabaseTransaction {
        self.inner().transaction()
    }

    /// Explicitly roll back the test transaction.
    ///
    /// Dropping the wrapper also drops the underlying SQLx transaction, but explicit
    /// rollback surfaces rollback failures to the test.
    pub async fn rollback(mut self) -> Result<()> {
        self.transaction
            .take()
            .expect("database test transaction is available")
            .rollback()
            .await
    }

    fn inner(&self) -> &AppTransaction {
        self.transaction
            .as_ref()
            .expect("database test transaction has already been completed")
    }
}

#[async_trait]
impl QueryExecutor for DatabaseTestTransaction {
    async fn raw_query_with(
        &self,
        sql: &str,
        bindings: &[crate::database::DbValue],
        options: QueryExecutionOptions,
    ) -> Result<Vec<DbRecord>> {
        self.inner().raw_query_with(sql, bindings, options).await
    }

    async fn raw_execute_with(
        &self,
        sql: &str,
        bindings: &[crate::database::DbValue],
        options: QueryExecutionOptions,
    ) -> Result<u64> {
        self.inner().raw_execute_with(sql, bindings, options).await
    }
}

impl AfterCommitSink for DatabaseTestTransaction {
    fn supports_after_commit(&self) -> bool {
        self.inner().supports_after_commit()
    }

    fn defer_after_commit(&self, callback: AfterCommitCallback) {
        self.inner().defer_after_commit(callback);
    }
}

impl ModelWriteExecutor for DatabaseTestTransaction {
    fn app_context(&self) -> &AppContext {
        self.inner().app_context()
    }

    fn active_transaction(&self) -> Option<&DatabaseTransaction> {
        Some(self.inner().transaction())
    }

    fn actor(&self) -> Option<&crate::auth::Actor> {
        self.inner().actor()
    }
}

/// Assert that a typed model query has at least one matching row.
pub async fn assert_database_has<M, E>(executor: &E, query: ModelQuery<M>) -> Result<()>
where
    M: Model,
    E: QueryExecutor,
{
    if query.exists(executor).await? {
        Ok(())
    } else {
        Err(Error::message(format!(
            "database assertion failed: expected a matching `{}` row",
            std::any::type_name::<M>()
        )))
    }
}

/// Assert that a typed model query has no matching row.
pub async fn assert_database_missing<M, E>(executor: &E, query: ModelQuery<M>) -> Result<()>
where
    M: Model,
    E: QueryExecutor,
{
    if query.doesnt_exist(executor).await? {
        Ok(())
    } else {
        Err(Error::message(format!(
            "database assertion failed: expected no matching `{}` row",
            std::any::type_name::<M>()
        )))
    }
}

/// Assert the exact number of rows matched by a typed model query.
pub async fn assert_database_count<M, E>(
    executor: &E,
    query: ModelQuery<M>,
    expected: u64,
) -> Result<()>
where
    M: Model,
    E: QueryExecutor,
{
    let actual = query.count(executor).await?;
    if actual == expected {
        Ok(())
    } else {
        Err(Error::message(format!(
            "database assertion failed: expected {expected} `{}` row(s), found {actual}",
            std::any::type_name::<M>()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{DbValue, Model};

    struct AssertionExecutor {
        exists: bool,
        count: i64,
    }

    #[async_trait]
    impl QueryExecutor for AssertionExecutor {
        async fn raw_query_with(
            &self,
            sql: &str,
            _bindings: &[DbValue],
            _options: QueryExecutionOptions,
        ) -> Result<Vec<DbRecord>> {
            if sql.contains("__foundry_count") {
                let mut record = DbRecord::new();
                record.insert("__foundry_count", DbValue::Int64(self.count));
                Ok(vec![record])
            } else if self.exists {
                Ok(vec![DbRecord::new()])
            } else {
                Ok(Vec::new())
            }
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

    #[derive(Debug, crate::Model)]
    #[foundry(table = "testing_assertion_records", primary_key_strategy = "manual")]
    struct AssertionRecord {
        id: i64,
    }

    #[tokio::test]
    async fn typed_database_assertions_report_matching_state() {
        let present = AssertionExecutor {
            exists: true,
            count: 2,
        };
        assert_database_has(&present, AssertionRecord::model_query())
            .await
            .unwrap();
        assert_database_count(&present, AssertionRecord::model_query(), 2)
            .await
            .unwrap();

        let missing = AssertionExecutor {
            exists: false,
            count: 0,
        };
        assert_database_missing(&missing, AssertionRecord::model_query())
            .await
            .unwrap();

        let error = assert_database_count(&present, AssertionRecord::model_query(), 1)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("expected 1"));
        assert!(error.to_string().contains("found 2"));
    }
}
