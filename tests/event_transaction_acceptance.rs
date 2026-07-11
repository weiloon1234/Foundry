use std::fs;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use foundry::prelude::*;

const EVENT_GUARD: GuardId = GuardId::new("event-test");

#[derive(Clone, Serialize)]
struct TransactionEvent {
    label: &'static str,
    fail: bool,
}

impl Event for TransactionEvent {
    const ID: EventId = EventId::new("tests.transaction_event");
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct EventRecord {
    label: &'static str,
    actor_id: Option<String>,
    actor_guard: Option<GuardId>,
    request_id: Option<String>,
    ip: Option<std::net::IpAddr>,
    user_agent: Option<String>,
}

struct RecordingTransactionListener {
    records: Arc<Mutex<Vec<EventRecord>>>,
}

#[async_trait]
impl EventListener<TransactionEvent> for RecordingTransactionListener {
    async fn handle(&self, context: &EventContext, event: &TransactionEvent) -> Result<()> {
        self.records.lock().unwrap().push(EventRecord {
            label: event.label,
            actor_id: context.actor().map(|actor| actor.id.clone()),
            actor_guard: context.actor().map(|actor| actor.guard.clone()),
            request_id: context.request_id().map(ToOwned::to_owned),
            ip: context.ip(),
            user_agent: context.user_agent().map(ToOwned::to_owned),
        });

        if event.fail {
            Err(Error::message("transaction event listener failed"))
        } else {
            Ok(())
        }
    }
}

struct TransactionEventProvider {
    records: Arc<Mutex<Vec<EventRecord>>>,
}

#[async_trait]
impl ServiceProvider for TransactionEventProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.register_guard(
            EVENT_GUARD,
            StaticBearerAuthenticator::new()
                .token("event-token", Actor::new("request-actor", EVENT_GUARD)),
        )?;
        registrar.listen_event::<TransactionEvent, _>(RecordingTransactionListener {
            records: self.records.clone(),
        })
    }
}

fn event_routes(registrar: &mut HttpRegistrar) -> Result<()> {
    registrar.scope("/events", |routes| {
        routes.guard(EVENT_GUARD);
        routes.post("/commit", "commit", commit_event, |_| {});
        routes.post("/rollback", "rollback", rollback_event, |_| {});
        routes.post(
            "/listener-failure",
            "listener_failure",
            failing_event,
            |_| {},
        );
        Ok(())
    })?;
    Ok(())
}

async fn commit_event(State(app): State<AppContext>, _actor: CurrentActor) -> Result<StatusCode> {
    let mut transaction = app.begin_transaction().await?;
    transaction.set_actor(Actor::new("captured-actor", GuardId::new("transaction")));
    transaction.dispatch_event_after_commit(TransactionEvent {
        label: "committed",
        fail: false,
    });
    transaction.set_actor(Actor::new("changed-after-buffer", GuardId::new("changed")));
    transaction.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn rollback_event(State(app): State<AppContext>, _actor: CurrentActor) -> Result<StatusCode> {
    let transaction = app.begin_transaction().await?;
    transaction.dispatch_event_after_commit(TransactionEvent {
        label: "rolled-back",
        fail: false,
    });
    transaction.rollback().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn failing_event(State(app): State<AppContext>, _actor: CurrentActor) -> Result<StatusCode> {
    let transaction = app.begin_transaction().await?;
    transaction.dispatch_event_after_commit(TransactionEvent {
        label: "listener-failed",
        fail: true,
    });
    transaction.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[tokio::test]
async fn transactional_events_dispatch_only_after_commit_with_captured_origin() {
    let Some(database_url) = std::env::var("FOUNDRY_TEST_POSTGRES_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };

    let config_dir = tempfile::tempdir().unwrap();
    fs::write(
        config_dir.path().join("00-database.toml"),
        format!("[database]\nurl = \"{database_url}\"\n"),
    )
    .unwrap();

    let records = Arc::new(Mutex::new(Vec::new()));
    let app = TestApp::builder()
        .load_config_dir(config_dir.path())
        .register_provider(TransactionEventProvider {
            records: records.clone(),
        })
        .register_middleware(TrustedProxy::new().trust_all().build())
        .register_routes(event_routes)
        .build()
        .await
        .unwrap();

    let response = app
        .client()
        .post("/events/commit")
        .bearer_auth("event-token")
        .header("x-request-id", "req-transaction-event")
        .header("x-forwarded-for", "203.0.113.42")
        .header("user-agent", "FoundryEventTest/1.0")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    assert_eq!(
        records.lock().unwrap().as_slice(),
        [EventRecord {
            label: "committed",
            actor_id: Some("captured-actor".to_string()),
            actor_guard: Some(GuardId::new("transaction")),
            request_id: Some("req-transaction-event".to_string()),
            ip: Some("203.0.113.42".parse().unwrap()),
            user_agent: Some("FoundryEventTest/1.0".to_string()),
        }]
    );

    let response = app
        .client()
        .post("/events/rollback")
        .bearer_auth("event-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(records.lock().unwrap().len(), 1);

    let response = app
        .client()
        .post("/events/listener-failure")
        .bearer_auth("event-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(records.lock().unwrap().len(), 2);
    assert_eq!(records.lock().unwrap()[1].label, "listener-failed");
}
