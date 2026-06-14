use std::collections::HashMap;

use crate::database::{Column, ColumnRef, DbValue, IntoFieldValue, Model, ModelWriteExecutor};
use crate::foundation::Result;

/// Trait for defining model factories with default values for testing.
///
/// ```ignore
/// impl Factory for User {
///     fn definition() -> Vec<FactoryValue<Self>> {
///         vec![
///             FactoryValue::new(User::EMAIL, format!("user-{}@test.com", Token::hex(4).unwrap())),
///             FactoryValue::new(User::NAME, "Test User"),
///             FactoryValue::new(User::ACTIVE, true),
///         ]
///     }
/// }
/// ```
pub trait Factory: Model {
    /// Define default column values for this model.
    fn definition() -> Vec<FactoryValue<Self>>;
}

#[derive(Clone)]
pub struct FactoryValue<M: Model> {
    column: ColumnRef,
    value: DbValue,
    _marker: std::marker::PhantomData<fn() -> M>,
}

impl<M: Model> FactoryValue<M> {
    pub fn new<T, V>(column: Column<M, T>, value: V) -> Self
    where
        V: IntoFieldValue<T>,
    {
        Self {
            column: column.column_ref(),
            value: value.into_field_value(column.db_type()),
            _marker: std::marker::PhantomData,
        }
    }

    fn name(&self) -> &str {
        &self.column.name
    }
}

/// Builder for creating model instances from factory definitions.
pub struct FactoryBuilder<M: Factory> {
    overrides: HashMap<String, FactoryValue<M>>,
    count: usize,
    _phantom: std::marker::PhantomData<M>,
}

impl<M: Factory> FactoryBuilder<M> {
    pub fn new() -> Self {
        Self {
            overrides: HashMap::new(),
            count: 1,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Override a specific column value.
    pub fn set<T, V>(mut self, column: Column<M, T>, value: V) -> Self
    where
        V: IntoFieldValue<T>,
    {
        let value = FactoryValue::new(column, value);
        self.overrides.insert(value.name().to_string(), value);
        self
    }

    /// Create multiple instances.
    pub fn count(mut self, n: usize) -> Self {
        self.count = n;
        self
    }

    /// Build the final column values (defaults merged with overrides).
    fn build_values(&self) -> Vec<FactoryValue<M>> {
        let defaults = M::definition();
        let mut values: Vec<FactoryValue<M>> = defaults
            .into_iter()
            .map(|value| {
                if let Some(override_val) = self.overrides.get(value.name()) {
                    override_val.clone()
                } else {
                    value
                }
            })
            .collect();

        // Add any overrides that aren't in defaults
        for value in self.overrides.values() {
            if !values
                .iter()
                .any(|existing| existing.name() == value.name())
            {
                values.push(value.clone());
            }
        }

        values
    }

    /// Insert one or more records into the database and return them.
    pub async fn create<E>(&self, executor: &E) -> Result<Vec<M>>
    where
        E: ModelWriteExecutor,
    {
        let mut results = Vec::with_capacity(self.count);

        for _ in 0..self.count {
            let mut create = M::model_create();
            for value in self.build_values() {
                create = create.set_column_value(value.column, value.value);
            }
            results.push(create.save(executor).await?);
        }

        Ok(results)
    }

    /// Insert a single record and return it.
    pub async fn create_one<E>(&self, executor: &E) -> Result<M>
    where
        E: ModelWriteExecutor,
    {
        let mut results = self.create(executor).await?;
        results
            .pop()
            .ok_or_else(|| crate::foundation::Error::message("factory create returned no rows"))
    }
}

impl<M: Factory> Default for FactoryBuilder<M> {
    fn default() -> Self {
        Self::new()
    }
}
