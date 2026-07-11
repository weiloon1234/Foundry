use std::fs;

use foundry::prelude::*;

const RECORDS_TABLE: &str = "foundry_testing_layer_records";

#[derive(Debug, foundry::Model)]
#[foundry(
    table = RECORDS_TABLE,
    primary_key_strategy = "manual",
    timestamps = false,
    audit = false
)]
struct TestingRecord {
    id: i64,
    parent_id: i64,
    label: String,
}

impl Factory for TestingRecord {
    fn definition() -> Vec<FactoryValue<Self>> {
        vec![
            FactoryValue::new(Self::ID, 1_i64),
            FactoryValue::new(Self::PARENT_ID, 0_i64),
            FactoryValue::new(Self::LABEL, "default"),
        ]
    }
}

#[tokio::test]
async fn database_test_transaction_rolls_back_factories_and_typed_assertions() {
    let Some(url) = std::env::var("FOUNDRY_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("00-testing.toml"),
        format!(
            r#"
            [database]
            url = "{url}"
            "#
        ),
    )
    .unwrap();
    let app = TestApp::builder()
        .load_config_dir(directory.path())
        .build()
        .await
        .unwrap();
    let database = app.app().database().unwrap();
    database
        .raw_execute(
            &format!(
                "CREATE TABLE IF NOT EXISTS {RECORDS_TABLE} (id BIGINT PRIMARY KEY, parent_id BIGINT NOT NULL, label TEXT NOT NULL)"
            ),
            &[],
        )
        .await
        .unwrap();
    database
        .raw_execute(&format!("TRUNCATE TABLE {RECORDS_TABLE}"), &[])
        .await
        .unwrap();

    let transaction = app.begin_database_test().await.unwrap();
    let created = TestingRecord::factory()
        .state([FactoryValue::new(TestingRecord::LABEL, "active")])
        .for_parent(TestingRecord::PARENT_ID, 42_i64)
        .sequence(|index| [FactoryValue::new(TestingRecord::ID, index as i64 + 100)])
        .count(2)
        .create(&transaction)
        .await
        .unwrap();
    assert_eq!(created.len(), 2);
    assert_database_has(
        &transaction,
        TestingRecord::model_query().where_(TestingRecord::PARENT_ID.eq(42_i64)),
    )
    .await
    .unwrap();
    assert_database_count(&transaction, TestingRecord::model_query(), 2)
        .await
        .unwrap();
    transaction.rollback().await.unwrap();

    assert_database_missing(database.as_ref(), TestingRecord::model_query())
        .await
        .unwrap();
    database
        .raw_execute(&format!("DROP TABLE {RECORDS_TABLE}"), &[])
        .await
        .unwrap();
    app.shutdown().await.unwrap();
}
