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

    fn factory() -> FactoryBuilder<Self> {
        FactoryBuilder::new()
    }
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
type FactorySequence<M> = Box<dyn Fn(usize) -> Vec<FactoryValue<M>> + Send + Sync>;

pub struct FactoryBuilder<M: Factory> {
    overrides: HashMap<String, FactoryValue<M>>,
    sequence: Option<FactorySequence<M>>,
    count: usize,
    _phantom: std::marker::PhantomData<M>,
}

impl<M: Factory> FactoryBuilder<M> {
    pub fn new() -> Self {
        Self {
            overrides: HashMap::new(),
            sequence: None,
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

    /// Apply a reusable factory state. Later states and explicit values replace
    /// earlier values for the same typed column.
    pub fn state<I>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = FactoryValue<M>>,
    {
        for value in values {
            self.overrides.insert(value.name().to_string(), value);
        }
        self
    }

    /// Set a typed belongs-to foreign key from a previously created parent.
    pub fn for_parent<T, V>(self, foreign_key: Column<M, T>, parent_key: V) -> Self
    where
        V: IntoFieldValue<T>,
    {
        self.set(foreign_key, parent_key)
    }

    /// Vary factory values by zero-based creation index.
    pub fn sequence<F, I>(mut self, sequence: F) -> Self
    where
        F: Fn(usize) -> I + Send + Sync + 'static,
        I: IntoIterator<Item = FactoryValue<M>>,
    {
        self.sequence = Some(Box::new(move |index| sequence(index).into_iter().collect()));
        self
    }

    /// Create multiple instances.
    pub fn count(mut self, n: usize) -> Self {
        self.count = n;
        self
    }

    /// Build the final column values (defaults merged with overrides).
    fn build_values(&self, index: usize) -> Vec<FactoryValue<M>> {
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

        if let Some(sequence) = &self.sequence {
            for value in sequence(index) {
                if let Some(existing) = values
                    .iter_mut()
                    .find(|existing| existing.name() == value.name())
                {
                    *existing = value;
                } else {
                    values.push(value);
                }
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

        for index in 0..self.count {
            let mut create = M::model_create();
            for value in self.build_values(index) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, crate::Model)]
    #[foundry(table = "testing_factory_records", primary_key_strategy = "manual")]
    struct FactoryRecord {
        id: i64,
        parent_id: i64,
        label: String,
    }

    impl Factory for FactoryRecord {
        fn definition() -> Vec<FactoryValue<Self>> {
            vec![
                FactoryValue::new(Self::ID, 1_i64),
                FactoryValue::new(Self::PARENT_ID, 0_i64),
                FactoryValue::new(Self::LABEL, "default"),
            ]
        }
    }

    fn value<'a>(values: &'a [FactoryValue<FactoryRecord>], name: &str) -> &'a DbValue {
        &values
            .iter()
            .find(|value| value.name() == name)
            .unwrap_or_else(|| panic!("factory value `{name}` is missing"))
            .value
    }

    #[test]
    fn states_parent_keys_and_sequences_merge_in_order() {
        let builder = FactoryRecord::factory()
            .state([FactoryValue::new(FactoryRecord::LABEL, "active")])
            .for_parent(FactoryRecord::PARENT_ID, 42_i64)
            .sequence(|index| {
                [
                    FactoryValue::new(FactoryRecord::ID, index as i64 + 10),
                    FactoryValue::new(FactoryRecord::LABEL, format!("record-{index}")),
                ]
            });

        let first = builder.build_values(0);
        let second = builder.build_values(1);

        assert_eq!(value(&first, "id"), &DbValue::Int64(10));
        assert_eq!(value(&first, "parent_id"), &DbValue::Int64(42));
        assert_eq!(value(&first, "label"), &DbValue::Text("record-0".into()));
        assert_eq!(value(&second, "id"), &DbValue::Int64(11));
        assert_eq!(value(&second, "label"), &DbValue::Text("record-1".into()));
    }
}
