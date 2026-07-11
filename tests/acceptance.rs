use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use foundry::prelude::*;
use tempfile::tempdir;

mod app {
    use super::*;

    pub mod ids {
        use super::*;

        pub const MOBILE_RULE: ValidationRuleId = ValidationRuleId::new("mobile");
        pub const HELLO_COMMAND: CommandId = CommandId::new("hello");
        pub const HEARTBEAT_SCHEDULE: ScheduleId = ScheduleId::new("heartbeat");
    }

    pub mod providers {
        use super::*;

        #[derive(Clone)]
        pub struct AppServiceProvider {
            pub events: Arc<Mutex<Vec<String>>>,
        }

        #[async_trait]
        impl ServiceProvider for AppServiceProvider {
            async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
                registrar.singleton_arc(self.events.clone())?;
                Ok(())
            }

            async fn boot(&self, app: &AppContext) -> Result<()> {
                let events = app.resolve::<Mutex<Vec<String>>>()?;
                events.lock().unwrap().push("provider:boot".to_string());
                Ok(())
            }
        }
    }

    pub mod validation {
        use super::*;

        pub struct MobileRule;

        #[async_trait]
        impl ValidationRule for MobileRule {
            async fn validate(
                &self,
                _context: &RuleContext,
                value: &str,
            ) -> std::result::Result<(), ValidationError> {
                if value.starts_with('+') && value[1..].chars().all(|ch| ch.is_ascii_digit()) {
                    Ok(())
                } else {
                    Err(ValidationError::new("mobile", "invalid mobile number"))
                }
            }
        }
    }

    pub mod portals {
        use super::*;
        use foundry::Validate;

        #[derive(Debug, Deserialize)]
        pub struct CreateUser {
            pub email: String,
            pub phone: String,
        }

        #[async_trait]
        impl RequestValidator for CreateUser {
            async fn validate(&self, validator: &mut Validator) -> Result<()> {
                validator
                    .field("email", self.email.clone())
                    .required()
                    .email()
                    .apply()
                    .await?;
                validator
                    .field("phone", self.phone.clone())
                    .required()
                    .rule(ids::MOBILE_RULE)
                    .apply()
                    .await
            }
        }

        #[async_trait]
        impl foundry::validation::FromMultipart for CreateUser {
            async fn from_multipart(
                multipart: &mut axum::extract::Multipart,
            ) -> foundry::foundation::Result<Self> {
                let mut email = None;
                let mut phone = None;
                while let Some(field) = multipart.next_field().await.map_err(|e| {
                    foundry::foundation::Error::message(format!("multipart error: {e}"))
                })? {
                    match field.name().unwrap_or("") {
                        "email" => {
                            email = Some(field.text().await.map_err(|e| {
                                foundry::foundation::Error::message(format!("field error: {e}"))
                            })?)
                        }
                        "phone" => {
                            phone = Some(field.text().await.map_err(|e| {
                                foundry::foundation::Error::message(format!("field error: {e}"))
                            })?)
                        }
                        _ => {}
                    }
                }
                Ok(Self {
                    email: email.unwrap_or_default(),
                    phone: phone.unwrap_or_default(),
                })
            }
        }

        #[derive(Debug, Deserialize, Validate)]
        pub struct CreateJsonUser {
            #[validate(required, email)]
            pub email: String,
            #[validate(required, min(8))]
            pub password: String,
        }

        #[derive(Debug, Deserialize, Validate)]
        #[serde(rename_all = "camelCase")]
        pub struct CreateTypedMultipartProfile {
            #[validate(required, min(2))]
            pub display_name: String,
            pub settings: serde_json::Value,
            pub metadata: Option<serde_json::Value>,
            pub age: Option<i32>,
            #[validate(distinct)]
            pub tags: Vec<String>,
            pub scores: Vec<i32>,
        }

        pub fn router(registrar: &mut HttpRegistrar) -> Result<()> {
            registrar.route("/health", get(health));
            registrar.route("/users", post(create_user));
            registrar.route("/json-users", post(create_json_user));
            registrar.route("/typed-multipart", post(create_typed_multipart_profile));
            Ok(())
        }

        async fn health(State(app): State<AppContext>) -> impl IntoResponse {
            let events = app.resolve::<Mutex<Vec<String>>>().unwrap();

            Json(serde_json::json!({
                "status": "ok",
                "events": events.lock().unwrap().clone(),
            }))
        }

        async fn create_user(Validated(payload): Validated<CreateUser>) -> impl IntoResponse {
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "email": payload.email,
                    "phone": payload.phone,
                })),
            )
        }

        async fn create_json_user(
            JsonValidated(payload): JsonValidated<CreateJsonUser>,
        ) -> impl IntoResponse {
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "email": payload.email,
                })),
            )
        }

        async fn create_typed_multipart_profile(
            Validated(payload): Validated<CreateTypedMultipartProfile>,
        ) -> impl IntoResponse {
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "name": payload.display_name,
                    "settings": payload.settings,
                    "metadata": payload.metadata,
                    "age": payload.age,
                    "tags": payload.tags,
                    "scores": payload.scores,
                })),
            )
        }
    }

    pub mod commands {
        use super::*;

        pub fn register(registry: &mut CommandRegistry) -> Result<()> {
            registry.command(
                ids::HELLO_COMMAND,
                Command::new("hello").about("test command"),
                |invocation: CommandInvocation| async move {
                    let events = invocation.app().resolve::<Mutex<Vec<String>>>()?;
                    events.lock().unwrap().push("command:hello".to_string());
                    Ok(())
                },
            )?;
            Ok(())
        }
    }

    pub mod schedules {
        use super::*;

        pub fn register(registry: &mut ScheduleRegistry) -> Result<()> {
            registry.cron(
                ids::HEARTBEAT_SCHEDULE,
                CronExpression::parse("*/1 * * * * *")?,
                |invocation| async move {
                    let events = invocation.app().resolve::<Mutex<Vec<String>>>()?;
                    events
                        .lock()
                        .unwrap()
                        .push("schedule:heartbeat".to_string());
                    Ok(())
                },
            )?;
            Ok(())
        }
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn write_config(dir: &Path, port: u16) {
    fs::write(
        dir.join("00-server.toml"),
        format!(
            r#"
            [server]
            host = "127.0.0.1"
            port = {port}
        "#
        ),
    )
    .unwrap();
}

fn write_i18n_config(dir: &Path) {
    let locales_dir = dir.join("locales");
    let en_dir = locales_dir.join("en");
    let ms_dir = locales_dir.join("ms");

    fs::create_dir_all(&en_dir).unwrap();
    fs::create_dir_all(&ms_dir).unwrap();

    fs::write(
        en_dir.join("validation.json"),
        r#"{
            "validation": {
                "invalid_request_body": "The request body is invalid.",
                "multipart_not_supported": "Multipart form-data is not supported for this endpoint."
            }
        }"#,
    )
    .unwrap();

    fs::write(
        ms_dir.join("validation.json"),
        r#"{
            "validation": {
                "invalid_request_body": "Badan permintaan tidak sah.",
                "multipart_not_supported": "Multipart form-data tidak disokong untuk endpoint ini."
            }
        }"#,
    )
    .unwrap();

    fs::write(
        dir.join("10-i18n.toml"),
        format!(
            r#"
            [i18n]
            default_locale = "en"
            fallback_locale = "en"
            resource_path = "{}"
        "#,
            locales_dir.display()
        ),
    )
    .unwrap();
}

fn build_app(config_dir: &Path, events: Arc<Mutex<Vec<String>>>) -> AppBuilder {
    App::builder()
        .load_config_dir(config_dir)
        .register_provider(app::providers::AppServiceProvider { events })
        .register_routes(app::portals::router)
        .register_commands(app::commands::register)
        .register_schedule(app::schedules::register)
        .register_validation_rule(app::ids::MOBILE_RULE, app::validation::MobileRule)
}

#[tokio::test]
async fn run_http_async_serves_routes_and_validation() {
    let config_dir = tempdir().unwrap();
    let port = free_port();
    write_config(config_dir.path(), port);

    let events = Arc::new(Mutex::new(Vec::new()));
    let task = tokio::spawn({
        let builder = build_app(config_dir.path(), events.clone());
        async move { builder.run_http_async().await.unwrap() }
    });

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}");

    for _ in 0..30 {
        if client.get(format!("{url}/health")).send().await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let health = client.get(format!("{url}/health")).send().await.unwrap();
    assert_eq!(health.status(), reqwest::StatusCode::OK);
    let payload: serde_json::Value = health.json().await.unwrap();
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["events"][0], "provider:boot");

    let invalid = client
        .post(format!("{url}/users"))
        .json(&serde_json::json!({
            "email": "not-an-email",
            "phone": "1234",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);

    let valid = client
        .post(format!("{url}/users"))
        .json(&serde_json::json!({
            "email": "foundry@example.com",
            "phone": "+60123456789",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(valid.status(), reqwest::StatusCode::CREATED);

    task.abort();
}

#[tokio::test]
async fn run_http_async_supports_json_only_validated_requests() {
    let config_dir = tempdir().unwrap();
    let port = free_port();
    write_config(config_dir.path(), port);

    let events = Arc::new(Mutex::new(Vec::new()));
    let task = tokio::spawn({
        let builder = build_app(config_dir.path(), events);
        async move { builder.run_http_async().await.unwrap() }
    });

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}");

    for _ in 0..30 {
        if client.get(format!("{url}/health")).send().await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let valid = client
        .post(format!("{url}/json-users"))
        .json(&serde_json::json!({
            "email": "json@example.com",
            "password": "supersecret",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(valid.status(), reqwest::StatusCode::CREATED);

    let multipart = client
        .post(format!("{url}/json-users"))
        .multipart(
            reqwest::multipart::Form::new()
                .text("email", "json@example.com")
                .text("password", "supersecret"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(
        multipart.status(),
        reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE
    );

    task.abort();
}

#[tokio::test]
async fn run_http_async_translates_json_only_request_rejections() {
    let config_dir = tempdir().unwrap();
    let port = free_port();
    write_config(config_dir.path(), port);
    write_i18n_config(config_dir.path());

    let events = Arc::new(Mutex::new(Vec::new()));
    let task = tokio::spawn({
        let builder = build_app(config_dir.path(), events);
        async move { builder.run_http_async().await.unwrap() }
    });

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}");

    for _ in 0..30 {
        if client.get(format!("{url}/health")).send().await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let invalid_json = client
        .post(format!("{url}/json-users"))
        .header("Accept-Language", "ms")
        .header("Content-Type", "application/json")
        .body("{")
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_json.status(), reqwest::StatusCode::BAD_REQUEST);
    let invalid_json_payload: serde_json::Value = invalid_json.json().await.unwrap();
    assert_eq!(
        invalid_json_payload["message"],
        "Badan permintaan tidak sah."
    );
    assert_eq!(invalid_json_payload["error_code"], "invalid_request_body");

    let multipart = client
        .post(format!("{url}/json-users"))
        .header("Accept-Language", "ms")
        .multipart(
            reqwest::multipart::Form::new()
                .text("email", "json@example.com")
                .text("password", "supersecret"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(
        multipart.status(),
        reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    let multipart_payload: serde_json::Value = multipart.json().await.unwrap();
    assert_eq!(
        multipart_payload["message"],
        "Multipart form-data tidak disokong untuk endpoint ini."
    );
    assert_eq!(multipart_payload["error_code"], "multipart_not_supported");

    task.abort();
}

#[tokio::test]
async fn run_http_async_parses_typed_multipart_fields() {
    let config_dir = tempdir().unwrap();
    let port = free_port();
    write_config(config_dir.path(), port);

    let events = Arc::new(Mutex::new(Vec::new()));
    let task = tokio::spawn({
        let builder = build_app(config_dir.path(), events);
        async move { builder.run_http_async().await.unwrap() }
    });

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}");

    for _ in 0..30 {
        if client.get(format!("{url}/health")).send().await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let response = client
        .post(format!("{url}/typed-multipart"))
        .multipart(
            reqwest::multipart::Form::new()
                .text("displayName", "Alice")
                .text("settings", r#"{"theme":"dark","layout":"stacked"}"#)
                .text("metadata", r#"{"source":"starter"}"#)
                .text("age", "42")
                .text("tags", "rust")
                .text("tags", "foundry")
                .text("scores", "10")
                .text("scores", "20"),
        )
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let payload: serde_json::Value = response.json().await.unwrap();
    assert_eq!(payload["name"], "Alice");
    assert_eq!(payload["settings"]["theme"], "dark");
    assert_eq!(payload["settings"]["layout"], "stacked");
    assert_eq!(payload["metadata"]["source"], "starter");
    assert_eq!(payload["age"], 42);
    assert_eq!(payload["tags"], serde_json::json!(["rust", "foundry"]));
    assert_eq!(payload["scores"], serde_json::json!([10, 20]));

    task.abort();
}

#[tokio::test]
async fn run_http_async_rejects_invalid_typed_multipart_values() {
    let config_dir = tempdir().unwrap();
    let port = free_port();
    write_config(config_dir.path(), port);

    let events = Arc::new(Mutex::new(Vec::new()));
    let task = tokio::spawn({
        let builder = build_app(config_dir.path(), events);
        async move { builder.run_http_async().await.unwrap() }
    });

    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}");

    for _ in 0..30 {
        if client.get(format!("{url}/health")).send().await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let invalid_json = client
        .post(format!("{url}/typed-multipart"))
        .multipart(
            reqwest::multipart::Form::new()
                .text("displayName", "Alice")
                .text("settings", "{")
                .text("age", "42"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_json.status(), reqwest::StatusCode::BAD_REQUEST);
    let invalid_json_payload: serde_json::Value = invalid_json.json().await.unwrap();
    assert_eq!(
        invalid_json_payload["message"],
        "field 'settings' has invalid JSON"
    );

    let invalid_number = client
        .post(format!("{url}/typed-multipart"))
        .multipart(
            reqwest::multipart::Form::new()
                .text("displayName", "Alice")
                .text("settings", r#"{"theme":"dark"}"#)
                .text("age", "not-a-number"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_number.status(), reqwest::StatusCode::BAD_REQUEST);
    let invalid_number_payload: serde_json::Value = invalid_number.json().await.unwrap();
    assert_eq!(
        invalid_number_payload["message"],
        "field 'age' has invalid value"
    );

    let duplicate_tags = client
        .post(format!("{url}/typed-multipart"))
        .multipart(
            reqwest::multipart::Form::new()
                .text("displayName", "Alice")
                .text("settings", r#"{"theme":"dark"}"#)
                .text("tags", "rust")
                .text("tags", "rust"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(
        duplicate_tags.status(),
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );
    let duplicate_tags_payload: serde_json::Value = duplicate_tags.json().await.unwrap();
    assert_eq!(duplicate_tags_payload["errors"][0]["field"], "tags");
    assert_eq!(duplicate_tags_payload["errors"][0]["code"], "distinct");

    task.abort();
}

#[tokio::test]
async fn cli_kernel_dispatches_registered_command() {
    let config_dir = tempdir().unwrap();
    write_config(config_dir.path(), free_port());

    let events = Arc::new(Mutex::new(Vec::new()));
    build_app(config_dir.path(), events.clone())
        .build_cli_kernel()
        .await
        .unwrap()
        .run_with_args(["foundry", "hello"])
        .await
        .unwrap();

    assert!(events
        .lock()
        .unwrap()
        .iter()
        .any(|entry| entry == "command:hello"));
}

#[tokio::test]
async fn scheduler_kernel_runs_registered_cron_jobs() {
    let config_dir = tempdir().unwrap();
    write_config(config_dir.path(), free_port());

    let events = Arc::new(Mutex::new(Vec::new()));
    let scheduler = build_app(config_dir.path(), events.clone())
        .build_scheduler_kernel()
        .await
        .unwrap();
    let now = DateTime::parse("2026-04-08T12:00:00Z").unwrap();

    let executed = scheduler.tick_at(now).await.unwrap();
    assert_eq!(executed, vec![app::ids::HEARTBEAT_SCHEDULE]);

    // The handler runs in a spawned task — yield to let it complete.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert!(events
        .lock()
        .unwrap()
        .iter()
        .any(|entry| entry == "schedule:heartbeat"));
}
