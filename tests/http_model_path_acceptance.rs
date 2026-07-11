use std::fs;

use foundry::prelude::*;
use foundry::testing::TestApp;
use serde_json::json;
use tempfile::tempdir;

const UUID_MODELS_TABLE: &str = "foundry_test_http_uuid_models";
const NUMERIC_MODELS_TABLE: &str = "foundry_test_http_numeric_models";
const TEXT_MODELS_TABLE: &str = "foundry_test_http_text_models";
const UUID_MODEL_ID: &str = "018f47aa-7c89-7f10-bf8f-9df04f572d86";
const MISSING_UUID_MODEL_ID: &str = "018f47aa-7c89-7f10-bf8f-9df04f572d87";

#[derive(Debug, foundry::Model)]
#[foundry(table = UUID_MODELS_TABLE)]
struct UuidRouteModel {
    id: ModelId<Self>,
    name: String,
}

#[derive(Debug, foundry::Model)]
#[foundry(table = NUMERIC_MODELS_TABLE, primary_key_strategy = "manual")]
struct NumericRouteModel {
    id: i64,
    name: String,
}

#[derive(Debug, foundry::Model)]
#[foundry(table = TEXT_MODELS_TABLE, primary_key_strategy = "manual")]
struct TextRouteModel {
    id: String,
    name: String,
}

fn model_routes(registrar: &mut HttpRegistrar) -> Result<()> {
    registrar.route("/models/uuid/{id}", get(show_uuid_model));
    registrar.route("/models/numeric/{id}", get(show_numeric_model));
    registrar.route("/models/text/{id}", get(show_text_model));
    Ok(())
}

async fn show_uuid_model(ModelPath(model): ModelPath<UuidRouteModel>) -> impl IntoResponse {
    Json(json!({ "id": model.id, "name": model.name }))
}

async fn show_numeric_model(ModelPath(model): ModelPath<NumericRouteModel>) -> impl IntoResponse {
    Json(json!({ "id": model.id, "name": model.name }))
}

async fn show_text_model(ModelPath(model): ModelPath<TextRouteModel>) -> impl IntoResponse {
    Json(json!({ "id": model.id, "name": model.name }))
}

async fn reset_model_tables(database: &DatabaseManager) {
    for statement in [
        format!("DROP TABLE IF EXISTS {UUID_MODELS_TABLE}"),
        format!("DROP TABLE IF EXISTS {NUMERIC_MODELS_TABLE}"),
        format!("DROP TABLE IF EXISTS {TEXT_MODELS_TABLE}"),
        format!("CREATE TABLE {UUID_MODELS_TABLE} (id UUID PRIMARY KEY, name TEXT NOT NULL)"),
        format!("CREATE TABLE {NUMERIC_MODELS_TABLE} (id BIGINT PRIMARY KEY, name TEXT NOT NULL)"),
        format!("CREATE TABLE {TEXT_MODELS_TABLE} (id TEXT PRIMARY KEY, name TEXT NOT NULL)"),
        format!(
            "INSERT INTO {UUID_MODELS_TABLE} (id, name) VALUES ('{UUID_MODEL_ID}', 'UUID model')"
        ),
        format!("INSERT INTO {NUMERIC_MODELS_TABLE} (id, name) VALUES (42, 'Numeric model')"),
        format!(
            "INSERT INTO {TEXT_MODELS_TABLE} (id, name) VALUES ('account-alpha', 'Text model')"
        ),
    ] {
        database.raw_execute(&statement, &[]).await.unwrap();
    }
}

async fn drop_model_tables(database: &DatabaseManager) {
    for statement in [
        format!("DROP TABLE IF EXISTS {UUID_MODELS_TABLE}"),
        format!("DROP TABLE IF EXISTS {NUMERIC_MODELS_TABLE}"),
        format!("DROP TABLE IF EXISTS {TEXT_MODELS_TABLE}"),
    ] {
        database.raw_execute(&statement, &[]).await.unwrap();
    }
}

#[tokio::test]
async fn model_path_resolves_typed_primary_keys_and_distinguishes_malformed_from_missing() {
    let Some(url) = std::env::var("FOUNDRY_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    let directory = tempdir().unwrap();
    fs::write(
        directory.path().join("foundry.toml"),
        format!("[database]\nurl = {url:?}\n"),
    )
    .unwrap();

    let app = TestApp::builder()
        .load_config_dir(directory.path())
        .register_routes(model_routes)
        .build()
        .await
        .unwrap();
    let database = app.app().database().unwrap();
    reset_model_tables(&database).await;
    let client = app.client();

    client
        .get(&format!("/models/uuid/{UUID_MODEL_ID}"))
        .send()
        .await
        .unwrap()
        .assert_json(&json!({ "id": UUID_MODEL_ID, "name": "UUID model" }));
    client
        .get("/models/numeric/42")
        .send()
        .await
        .unwrap()
        .assert_json(&json!({ "id": 42, "name": "Numeric model" }));
    client
        .get("/models/text/account-alpha")
        .send()
        .await
        .unwrap()
        .assert_json(&json!({ "id": "account-alpha", "name": "Text model" }));

    client
        .get("/models/uuid/not-a-uuid")
        .send()
        .await
        .unwrap()
        .assert_status(StatusCode::BAD_REQUEST);
    client
        .get("/models/numeric/not-a-number")
        .send()
        .await
        .unwrap()
        .assert_status(StatusCode::BAD_REQUEST);

    client
        .get(&format!("/models/uuid/{MISSING_UUID_MODEL_ID}"))
        .send()
        .await
        .unwrap()
        .assert_not_found();
    client
        .get("/models/numeric/404")
        .send()
        .await
        .unwrap()
        .assert_not_found();
    client
        .get("/models/text/account-missing")
        .send()
        .await
        .unwrap()
        .assert_not_found();

    drop_model_tables(&database).await;
    app.shutdown().await.unwrap();
}
