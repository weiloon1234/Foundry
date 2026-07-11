# Foundry Recipes

Copy-pasteable paths for common app work. The examples assume the split bootstrap layout used by Foundry-starter, but the same APIs work in smaller apps.

## Production Readiness

Run this before announcing a deployable build:

```bash
cargo run -- config:publish
cargo run -- env:publish
cargo run -- key:generate
cargo run -- migrate:publish
cargo run -- db:migrate --lock-timeout-ms 30000
cargo run -- doctor --deploy --strict
make verify-release
```

Expected result:

- `doctor --deploy --strict` exits `0`.
- The text verdict ends with `Production readiness: ready - deploy checks passed.`
- `make verify-release` passes formatting, tests, clippy, fixture checks, and package dry-run.

Use `cargo run -- doctor --deploy --json --strict` in deploy tooling.

## Authenticated CRUD Route

Register the guard, policy, and model-facing route:

```rust
use foundry::prelude::*;

const API_GUARD: GuardId = GuardId::new("api");
const USERS_WRITE: PermissionId = PermissionId::new("users:write");

#[derive(Debug, Deserialize, Validate)]
struct CreateUserRequest {
    #[validate(required, email)]
    email: String,
    #[validate(required, min(3))]
    name: String,
}

async fn create_user(
    State(app): State<AppContext>,
    CurrentActor(actor): CurrentActor,
    Validated(input): Validated<CreateUserRequest>,
) -> Result<impl IntoResponse> {
    let db = app.database()?;
    let user = User::model_create()
        .set(User::EMAIL, input.email)
        .set(User::NAME, input.name)
        .execute(&*db)
        .await?;

    Ok((StatusCode::CREATED, Json(UserResource::make(&user))))
}

fn routes(r: &mut HttpRegistrar) -> Result<()> {
    r.route_with_options(
        "/api/users",
        post(create_user),
        HttpRouteOptions::new().guard(API_GUARD).permission(USERS_WRITE),
    );
    Ok(())
}
```

Test the route through `TestApp` and assert status/body with `TestResponse`.

## Queued Email After Commit

Dispatch jobs only after the database commit when the job depends on committed rows:

```rust
#[derive(Debug, Serialize, Deserialize)]
struct SendWelcomeEmail {
    user_id: ModelId<User>,
}

#[async_trait]
impl Job for SendWelcomeEmail {
    const ID: JobId = JobId::new("send_welcome_email");

    async fn handle(&self, ctx: JobContext) -> Result<()> {
        let app = ctx.app();
        let email = app.email()?;
        email.send(EmailMessage::new("Welcome")
            .to("user@example.com")
            .text_body("Welcome to the app"))
            .await
    }
}

async fn register_user(app: &AppContext, input: CreateUserRequest) -> Result<()> {
    let mut tx = app.begin_transaction().await?;
    let user = User::model_create()
        .set(User::EMAIL, input.email)
        .set(User::NAME, input.name)
        .execute(tx.transaction())
        .await?;

    tx.dispatch_after_commit(SendWelcomeEmail { user_id: user.id });
    tx.commit().await
}
```

Register the job in a `ServiceProvider` with `registrar.register_job::<SendWelcomeEmail>()?`.

## Upload With Attachments And Images

Use validated multipart input, store the file, then attach it to a model:

```rust
async fn upload_avatar(
    State(app): State<AppContext>,
    CurrentActor(_actor): CurrentActor,
    form: MultipartForm,
) -> Result<impl IntoResponse> {
    let avatar = form.file("avatar")?;
    let user = current_user(&app).await?;

    Attachment::upload(avatar)
        .collection("avatar")
        .resize_to_fill(512, 512)
        .format(ImageFormat::WebP)
        .store(&app, User::attachable_type(), &user.attachable_id())
        .await?;

    Ok(Json(MessageResponse::ok()))
}
```

Prefer model-level attachment specs so upload limits and image policy stay close to the domain.

## Datatable With Filters And Export

Register the datatable once, then expose JSON and export routes:

```rust
struct UserDatatable;

impl Datatable for UserDatatable {
    type Row = User;
    type Query = ModelQuery<User>;

    const ID: &'static str = "users";

    fn query(_ctx: &DatatableContext) -> Self::Query {
        User::query()
    }

    fn columns() -> Vec<DatatableColumn<Self::Row>> {
        vec![
            DatatableColumn::field(User::EMAIL).label("Email").searchable().sortable(),
            DatatableColumn::field(User::NAME).label("Name").searchable().sortable(),
        ]
    }
}

async fn users_table(
    State(app): State<AppContext>,
    Json(request): Json<DatatableRequest>,
) -> Result<impl IntoResponse> {
    Ok(Json(UserDatatable::json(&app, None, request).await?))
}
```

For large exports, return `DatatableExportAccepted` and let Foundry queue the XLSX generation job.

## Plugin Extension

Use direct plugin registration for framework contributions:

```rust
struct AuditToolsPlugin;

impl Plugin for AuditToolsPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::new("audit-tools", Version::new(1, 0, 0), VersionReq::parse(">=0.1").unwrap())
            .description("Audit routes and cleanup commands")
    }

    fn register(&self, r: &mut PluginRegistrar) -> Result<()> {
        r.register_routes(audit_routes);
        r.register_commands(audit_commands);
        r.register_schedule(audit_schedules);
        Ok(())
    }
}
```

Keep plugin config under `[plugins.<plugin-id>]` and prefer typed IDs for commands, jobs, policies, channels, and schedules.
