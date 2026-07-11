# Testing Foundry Applications

Foundry's test harness boots the same providers, plugins, middleware, routes,
and configuration as a production application, then sends requests through the
Axum router in process. Tests do not need to bind a TCP port.

## Boot the application under test

Keep application assembly in one function and pass that same `AppBuilder` to
the runtime and the test harness:

```rust
use foundry::prelude::*;

#[tokio::test]
async fn health_endpoint_is_available() -> Result<()> {
    let app = TestApp::from_builder(build_app())
        .build()
        .await?;

    app.client()
        .get("/health")
        .send()
        .await?
        .assert_ok()
        .assert_json_path("status", &serde_json::json!("ok"));

    app.shutdown().await
}
```

Use `TestApp::builder()` when a test intentionally assembles a small isolated
application. `TestApp::from_builder(...)` is preferred for acceptance tests
because it prevents test-only bootstrap from drifting away from production.
Always await `shutdown()` in tests that start managed tasks or register plugins
so their shutdown hooks complete before the Tokio runtime exits.

If the test process installs a global tracing subscriber itself, make that
ownership explicit on the shared application builder (or on
`TestAppBuilder`) with `.use_external_tracing_subscriber()`. Bootstrap now
returns an error for an implicit subscriber conflict so a hosted test cannot
quietly lose Foundry logging configuration.

## Test a plugin in isolation

Plugin authors can use `PluginTestHarness` instead of assembling a separate
consumer application for each contract test. The harness still uses Foundry's
normal plugin registration, dependency resolution, config precedence, boot,
router construction, and shutdown paths:

```rust
#[tokio::test]
async fn plugin_registers_its_contract() -> Result<()> {
    let app = PluginTestHarness::new(MY_PLUGIN_ID, MyPlugin)
        .register_plugin(MyDependencyPlugin)
        .load_config_dir("tests/config")
        .configure(|builder| builder.register_provider(TestHostProvider))
        .build()
        .await?;

    assert_eq!(app.plugin_id(), &MY_PLUGIN_ID);
    assert_eq!(app.contributions().provider_count, 1);
    assert!(app.registry().plugin(&MY_DEPENDENCY_ID).is_some());
    let service = app.resolve::<MyPluginService>()?;
    assert!(service.is_ready());

    app.shutdown().await
}
```

The primary ID is checked after bootstrap, so an accidental manifest/test ID
mismatch fails clearly and the harness still runs shutdown. `manifest()`,
`contributions()`, and `registry()` expose registration metadata;
`app()`, `resolve()`, `client()`, and `test_app()` cover behavioral assertions.
Use `into_test_app()` when later test setup needs only the general `TestApp`
surface. See the [plugin guide](plugins.md#testing-a-plugin) for the shorter
author pattern. These testing APIs are additive and require no application
config change.

## Build and send requests

`TestClient` supports `GET`, `POST`, `PUT`, `PATCH`, and `DELETE`. Request
builders accept headers, bearer authentication, raw bytes, plain text, or JSON:

```rust
let response = app.client()
    .post("/api/users")
    .bearer_auth(&token)
    .header("x-request-id", "test-create-user")
    .json(&serde_json::json!({
        "name": "Ada Lovelace",
        "email": "ada@example.test"
    }))?
    .send()
    .await?;

response
    .assert_created()
    .assert_header("content-type", "application/json")
    .assert_json_shape(&["data.id", "data.name", "data.email"])
    .assert_json_path("data.name", &serde_json::json!("Ada Lovelace"))
    .assert_json_fragment(&serde_json::json!({
        "email": "ada@example.test"
    }));
```

The fluent response methods cover exact and class-level status checks, headers,
exact JSON, dot-separated object/array paths, JSON fragments and shapes,
validation fields, redirects, and attachment downloads. Accessors remain
available when a test needs custom assertions:

```rust
assert_eq!(response.status(), axum::http::StatusCode::CREATED);
let payload: serde_json::Value = response.json()?;
assert_eq!(payload["data"]["name"], "Ada Lovelace");
```

Validation responses use the framework's stable field contract:

```rust
app.client()
    .post("/api/users")
    .json(&serde_json::json!({}))?
    .send()
    .await?
    .assert_unprocessable()
    .assert_validation_error("email");
```

## Authenticate requests and freeze application time

Use `acting_as` when a test needs an authenticated actor without creating a
credential record. The actor's guard must match the route, and normal MFA,
permission, policy, authorization, and post-auth middleware checks still run:

```rust
let actor = Actor::new("user-1", GuardId::new("api"))
    .with_permissions([PermissionId::new("orders.read")]);

app.client()
    .acting_as(actor)
    .get("/api/orders")
    .send()
    .await?
    .assert_ok();
```

`TestClient::with_bearer_token` and `with_session` apply a credential to every
request from that client. `TestRequestBuilder::bearer_auth`, `session_auth`, and
`acting_as` provide per-request variants.

Application code that reads time through `AppContext::clock()` can use a
controllable clock:

```rust
let now = DateTime::parse("2026-07-11T12:00:00Z")?;
let clock = app.freeze_time(now)?;

assert_eq!(app.app().clock().now(), now);
clock.advance_seconds(60);
assert_eq!(app.app().clock().now(), now.add_seconds(60));
```

`ClockFake` does not replace direct `DateTime::now()` or third-party clock
reads. Use the application clock at domain boundaries that must be testable.

## Fake framework side effects

The first-party fakes share the same fluent record/reset/assert convention.
Installing one on `TestAppBuilder` suppresses its real side effect:

```rust
let events = EventFake::new();
let jobs = JobFake::new();
let mail = MailFake::new();
let notifications = NotificationFake::new();
let http = HttpClientFake::new();

let app = TestApp::from_builder(build_app())
    .fake_events(events.clone())
    .fake_jobs(jobs.clone())
    .fake_mail(mail.clone())
    .fake_notifications(notifications.clone())
    .fake_http(http.clone())
    .build()
    .await?;

events.assert_dispatched_where::<OrderCreated, _>(|event, origin| {
    event.order_id == order_id && origin.and_then(|value| value.actor.as_ref()).is_some()
});
jobs.assert_dispatched_where::<SendReceipt, _>(|job, record| {
    job.order_id == order_id && record.queue == QueueId::new("email")
});
mail.assert_sent_where(|message| message.subject == "Your receipt");
notifications.assert_sent("order_shipped");
```

`MailFake` records fully resolved `OutboundEmail` values. Queued mail is a job,
so assert it through `JobFake`. `NotificationFake` records both immediate and
queued intent before channel rendering/delivery. `HttpClientFake` retains its
queued response/error sequence and typed request assertions; an exhausted
sequence fails instead of reaching the network.

`StorageFake` is an in-memory `StorageAdapter`, not an implicit global
`StorageManager` replacement. Register its driver factory from a test provider
and select that driver on the test disk:

```rust
registrar.register_storage_driver("fake", storage_fake.driver_factory())?;
```

```toml
[storage]
default = "testing"

[storage.disks.testing]
driver = "fake"
visibility = "private"
```

This explicit config boundary keeps the same disk selection path used in
production. Use `assert_exists`, `assert_missing`, `assert_content`, and
`assert_written_count` on the retained `StorageFake` handle.

## Replace infrastructure with a test service

`replace_service` and `replace_service_arc` replace a service after the normal
bootstrap has registered it and before the router is built. This keeps the
production container strict while allowing an acceptance test to substitute a
fake implementation:

```rust
let fake_mail = MailSpy::new();
let app = TestApp::from_builder(build_app())
    .replace_service::<MailGateway>(fake_mail.clone())
    .build()
    .await?;
```

The concrete service type must already be registered by a provider. Attempting
to replace an absent service is a build error, which catches a fake wired under
the wrong type. Prefer a shared fake or spy from `foundry::testing` when one is
available; otherwise implement the application's existing service trait rather
than adding a second abstraction just for tests.

## Define typed model factories

Factories use model column constants, so overrides retain the model's field
types:

```rust
impl Factory for User {
    fn definition() -> Vec<FactoryValue<Self>> {
        vec![
            FactoryValue::new(User::EMAIL, "user@example.test"),
            FactoryValue::new(User::NAME, "Test User"),
            FactoryValue::new(User::ACTIVE, true),
        ]
    }
}

let user = FactoryBuilder::<User>::new()
    .set(User::EMAIL, "ada@example.test")
    .create_one(&database)
    .await?;

let reviewers = FactoryBuilder::<User>::new()
    .set(User::ACTIVE, true)
    .count(3)
    .create(&database)
    .await?;
```

Reusable states are typed `FactoryValue` collections. Sequences receive the
zero-based creation index, and `for_parent` sets a typed belongs-to key:

```rust
let reviewers = User::factory()
    .state(User::active_state())
    .for_parent(User::TEAM_ID, team.id)
    .sequence(|index| {
        [FactoryValue::new(
            User::EMAIL,
            format!("reviewer-{index}@example.test"),
        )]
    })
    .count(3)
    .create(&transaction)
    .await?;
```

Factories insert through `ModelWriteExecutor`, so a `DatabaseManager` or an
application transaction can be used as the executor.

## PostgreSQL test safety

Database acceptance tests use PostgreSQL, matching Foundry's supported driver.
Point `FOUNDRY_TEST_POSTGRES_URL` at a disposable database and guard any cleanup
that can drop or truncate data:

```rust
let database_url = std::env::var("FOUNDRY_TEST_POSTGRES_URL")?;
foundry::testing::assert_safe_to_wipe(&database_url)?;
```

`assert_safe_to_wipe` rejects unsafe-looking targets unless the database name
is clearly test-scoped. It is a final safety check, not a substitute for using
dedicated credentials and an isolated PostgreSQL database in CI.

Run the complete target matrix with database-backed branches enabled:

```bash
FOUNDRY_TEST_POSTGRES_URL=postgres://... make test-postgres
```

The CI and release-readiness workflows export that exact variable. A stale or
differently named variable causes optional test bodies to skip, so release
verification should always include this explicit command or an equivalent
`make verify-release` run with the variable exported.

For row-level isolation, begin a transaction from `TestApp`, use it for every
factory/query/assertion, and roll it back explicitly:

```rust
let transaction = app.begin_database_test().await?;

User::factory()
    .state(User::active_state())
    .create_one(&transaction)
    .await?;

assert_database_has(
    &transaction,
    User::model_query().where_(User::ACTIVE.eq(true)),
)
.await?;
assert_database_count(&transaction, User::model_query(), 1).await?;
assert_database_missing(
    &transaction,
    User::model_query().where_(User::EMAIL.eq("missing@example.test")),
)
.await?;

transaction.rollback().await?;
```

`DatabaseTestTransaction` implements `ModelWriteExecutor`, including lifecycle
context. Its rollback drops every deferred after-commit callback by design.
Use a separately committed test with isolated cleanup when the behavior under
test is the after-commit dispatch itself. Dropping the wrapper also lets SQLx
roll the transaction back, but explicit `rollback()` is preferred because it
surfaces rollback failures.

All testing additions are additive. They require no configuration or migration
unless a test opts into `StorageFake`, whose test disk must select the custom
driver as shown above.

## Test boundaries

- Use unit tests for pure rule, parser, compiler, and state-machine behavior.
- Use `TestApp` for HTTP, middleware, provider, plugin, and lifecycle behavior.
- Use a disposable PostgreSQL database for SQL compilation, transaction,
  relation, and persistence behavior.
- Exercise worker, scheduler, CLI, and WebSocket kernels directly when the
  runtime lifecycle itself is under test.
- Assert stable semantic IDs, error codes, and wire fields instead of display
  text that may be localized.

Run the normal repository gate before shipping:

```bash
make verify
```
