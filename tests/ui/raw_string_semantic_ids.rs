use async_trait::async_trait;
use foundry::prelude::*;

struct DummyRule;

#[async_trait]
impl ValidationRule for DummyRule {
    async fn validate(
        &self,
        _context: &RuleContext,
        _value: &str,
    ) -> std::result::Result<(), ValidationError> {
        Ok(())
    }
}

struct DummyGuard;

#[async_trait]
impl BearerAuthenticator for DummyGuard {
    async fn authenticate(&self, _token: &str) -> Result<Option<Actor>> {
        Ok(None)
    }
}

struct DummyPolicy;

#[async_trait]
impl Policy for DummyPolicy {
    async fn evaluate(&self, _actor: &Actor, _app: &AppContext) -> Result<bool> {
        Ok(true)
    }
}

struct DummyProvider;

#[async_trait]
impl ServiceProvider for DummyProvider {
    async fn register(&self, registrar: &mut ServiceRegistrar) -> Result<()> {
        registrar.register_guard("api", DummyGuard)?;
        registrar.register_policy("is_admin", DummyPolicy)?;
        Ok(())
    }
}

fn main() {
    let _ = App::builder()
        .register_provider(DummyProvider)
        .register_validation_rule("mobile", DummyRule);

    let _ = HttpRouteOptions::new()
        .guard("api")
        .permission("reports:view");

    let _ = WebSocketChannelOptions::new()
        .guard("api")
        .permission("ws:chat");

    let mut commands = CommandRegistry::new();
    let _ = commands.command(Command::new("hello"), |_invocation: CommandInvocation| async {
        Ok(())
    });

    let mut schedules = ScheduleRegistry::new();
    let _ = schedules.cron(
        "heartbeat",
        CronExpression::parse("*/1 * * * * *").unwrap(),
        |_invocation| async { Ok(()) },
    );
}
