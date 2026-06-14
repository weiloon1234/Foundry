# Database Guide

AST-first query system with typed models, relations, projections, lifecycle hooks, and migrations.

---

## Quick Start

```rust
#[derive(Model)]
#[foundry(table = "posts")]
struct Post {
    id: ModelId<Self>,
    title: String,
    body: String,
    published: bool,
    created_at: DateTime,
    updated_at: DateTime,
}

// Create
let post = Post::model_create()
    .set(Post::TITLE, "Hello World")
    .set(Post::BODY, "My first post")
    .set(Post::PUBLISHED, true)
    .save(&*app.database()?)
    .await?;

// Query
let posts = Post::model_query()
    .where_(Post::PUBLISHED.eq(true))
    .order_by(Post::CREATED_AT.desc())
    .all(&*app.database()?)
    .await?;

// Update
post.update()
    .set(Post::TITLE, "Updated Title")
    .save(&*app.database()?)
    .await?;

// Delete
post.delete().execute(&*app.database()?).await?;
```

---

## Models

### Defining a Model

```rust
#[derive(Model)]
#[foundry(table = "users", lifecycle = UserLifecycle, soft_deletes = true)]
struct User {
    id: ModelId<Self>,              // UUIDv7 primary key (auto-generated)
    email: String,
    name: String,
    #[foundry(write_mutator = "hash_password")]
    password: String,               // auto-hashed on write via mutator
    status: UserStatus,             // AppEnum stored as TEXT
    login_count: i64,
    nickname: Option<String>,       // nullable column
    orders: Loaded<Vec<Order>>,     // eager-loaded relation (not a DB column)
    order_count: Loaded<i64>,       // relation aggregate (not a DB column)
    created_at: DateTime,           // auto-managed timestamp
    updated_at: DateTime,           // auto-managed timestamp
    deleted_at: Option<DateTime>,   // soft delete marker
}
```

**What `#[derive(Model)]` generates:**

| Generated | Example |
|-----------|---------|
| Column constants | `User::EMAIL`, `User::NAME`, `User::STATUS` — typed `Column<User, T>` |
| Query builder | `User::model_query()` → `ModelQuery<User>` |
| Create builder | `User::model_create()` → `CreateModel<User>` |
| Bulk create | `User::model_create_many()` → `CreateManyModel<User>` |
| Update builder | `User::model_update()` → `UpdateModel<User>` |
| Delete builder | `User::model_delete()` → soft delete, `User::model_force_delete()` → permanent |
| Restore builder | `User::model_restore()` → restore soft-deleted |
| Instance methods | `user.update()`, `user.delete()`, `user.force_delete()`, `user.restore()` |
| Hydration | Builds `User` from `DbRecord` automatically |

### Model Attributes

| Attribute | Required | Default | Description |
|-----------|----------|---------|-------------|
| `#[foundry(table = "...")]` | Yes | — | Database table name |
| `lifecycle = Type` | No | No hooks | Struct implementing `ModelLifecycle<M>` |
| `audit = true/false` | No | `true` | Enable or disable the built-in audit writer for this model |
| `timestamps = true/false` | No | Config default | Auto-manage `created_at`/`updated_at` |
| `soft_deletes = true/false` | No | Config default | Enable soft deletes via `deleted_at` |
| `primary_key_strategy = "uuid_v7"` | No | `uuid_v7` | `uuid_v7` (auto) or `manual` |

### Field Attributes

| Attribute | Description |
|-----------|-------------|
| `#[foundry(write_mutator = "fn_name")]` | Transform value on create/update (e.g., hash password) |
| `#[foundry(read_accessor = "fn_name")]` | Transform value on read |
| `#[foundry(audit_exclude)]` | Exclude this persisted column from built-in audit payloads |
| `#[foundry(skip)]` | Skip this field in DB operations |

### ModelId\<M\>

Type-safe primary key (UUIDv7):

```rust
let id = ModelId::<User>::generate();  // generate new
let user = User::model_query().where_(User::ID.eq(id)).first(&*db).await?;
```

Serialized as a string (`"01912a4b-7c8d-7000-abcd-ef1234567890"`), stored as UUID in Postgres.

---

## Querying

### Basic Queries

```rust
let db = app.database()?;

// All rows
let users = User::model_query().all(&*db).await?;

// First match
let user = User::model_query()
    .where_(User::EMAIL.eq("alice@example.com"))
    .first(&*db)
    .await?;

// Count
let total = User::model_query().count(&*db).await?;

// Exists
let has_admins = User::model_query()
    .where_(User::STATUS.eq(UserStatus::Active))
    .exists(&*db)
    .await?;
```

### Finder Helpers

```rust
let user = User::model_query().find(&*db, user_id).await?;
let users = User::model_query().find_many(&*db, [first_id, second_id]).await?;

let required = User::model_query()
    .where_(User::EMAIL.eq("alice@example.com"))
    .first_or_fail(&*db)
    .await?;

let email: Option<String> = User::model_query()
    .where_(User::ID.eq(user_id))
    .value(&*db, User::EMAIL)
    .await?;

let no_pending = User::model_query()
    .where_(User::STATUS.eq(UserStatus::Pending))
    .doesnt_exist(&*db)
    .await?;
```

### Column Operators

```rust
User::EMAIL.eq("value")
User::EMAIL.not_eq("value")
User::LOGIN_COUNT.gt(10)
User::LOGIN_COUNT.gte(10)
User::LOGIN_COUNT.lt(100)
User::LOGIN_COUNT.lte(100)
User::EMAIL.ieq("alice@example.com")
User::EMAIL.like("%@example.com")
User::EMAIL.not_like("%spam%")
User::ID.in_list([id1, id2, id3])
User::NICKNAME.is_null()
User::NICKNAME.is_not_null()
```

### Ordering & Limits

```rust
User::model_query()
    .order_by(User::CREATED_AT.desc())
    .order_by(User::NAME.asc())
    .limit(10)
    .offset(20)
    .all(&*db).await?;
```

### Soft Deletes

```rust
// Default: excludes soft-deleted rows
User::model_query().all(&*db).await?;

// Include soft-deleted
User::model_query().with_trashed().all(&*db).await?;

// Only soft-deleted
User::model_query().only_trashed().all(&*db).await?;
```

### Pagination

**Offset-based:**

```rust
let page = User::model_query()
    .order_by(User::CREATED_AT.desc())
    .paginate(&*db, Pagination::new(1, 20))
    .await?;

page.data;         // Collection<User>
page.total;        // total matching rows
page.pagination;   // { page: 1, per_page: 20 }
```

**Cursor-based (for large datasets):**

```rust
let result = User::model_query()
    .order_by(User::ID.asc())
    .cursor_paginate(&*db, User::ID, CursorPagination::new(20))
    .await?;

result.data;           // Vec<User>
result.meta.has_next;  // bool
result.cursors.next;   // Option<String> — pass as .after() for next page
```

### Chunking

```rust
User::model_query()
    .order_by(User::ID.asc())
    .chunk(&*db, 500, |users| async move {
        // process Collection<User>
        Ok(())
    })
    .await?;

User::model_query()
    .chunk_by_id(&*db, User::ID, 500, |users| async move {
        // keyset pagination; safer for large mutable tables
        Ok(())
    })
    .await?;

User::model_query()
    .each_by_id(&*db, User::ID, 500, |user| async move {
        // process one model at a time
        Ok(())
    })
    .await?;
```

### Streaming

Process large result sets without loading everything into memory:

```rust
let mut stream = User::model_query()
    .order_by(User::ID.asc())
    .stream(&*db)?;

while let Some(user) = stream.next().await {
    let user = user?;
    // process one at a time
}
```

### Locking

```rust
// SELECT ... FOR UPDATE
let user = User::model_query()
    .where_(User::ID.eq(id))
    .for_update()
    .first(&*db)
    .await?;

// Skip locked rows (non-blocking)
let available = User::model_query()
    .where_(User::STATUS.eq(UserStatus::Pending))
    .for_update()
    .skip_locked()
    .first(&*db)
    .await?;
```

### Query Scopes

Reusable query filters:

```rust
impl User {
    fn active(q: ModelQuery<Self>) -> ModelQuery<Self> {
        q.where_(User::STATUS.eq(UserStatus::Active))
    }

    fn recent(q: ModelQuery<Self>) -> ModelQuery<Self> {
        q.order_by(User::CREATED_AT.desc()).limit(10)
    }
}

let users = User::model_query()
    .scope(User::active)
    .scope(User::recent)
    .all(&*db).await?;
```

---

## Creating

### Single Row

```rust
let user = User::model_create()
    .set(User::EMAIL, "alice@example.com")
    .set(User::NAME, "Alice")
    .set(User::PASSWORD, "secret")       // write_mutator auto-hashes
    .set(User::STATUS, UserStatus::Active)
    .save(&*db)
    .await?;
```

### Bulk Insert

```rust
let users = User::model_create_many()
    .row(|r| r.set(User::EMAIL, "a@example.com").set(User::NAME, "Alice"))
    .row(|r| r.set(User::EMAIL, "b@example.com").set(User::NAME, "Bob"))
    .row(|r| r.set(User::EMAIL, "c@example.com").set(User::NAME, "Charlie"))
    .get(&*db)
    .await?;  // returns Collection<User>

// Explicit fast path for imports/backfills that do not need lifecycle hooks/events.
// Model conventions, write mutators, validation, and audit recording still apply.
User::model_create_many()
    .row(|r| r.set(User::EMAIL, "import@example.com").set(User::NAME, "Import"))
    .without_lifecycle()
    .execute(&*db)
    .await?;
```

### Upsert (ON CONFLICT)

```rust
// Insert or update on conflict
User::model_create()
    .set(User::EMAIL, "alice@example.com")
    .set(User::NAME, "Alice Updated")
    .on_conflict_columns([User::EMAIL])
    .do_update()
    .set_excluded(User::NAME)      // use the value from the INSERT
    .save(&*db)
    .await?;

// Insert or ignore
User::model_create()
    .set(User::EMAIL, "alice@example.com")
    .on_conflict_columns([User::EMAIL])
    .do_nothing()
    .execute(&*db)
    .await?;
```

---

## Updating

### By Query

```rust
User::model_update()
    .set(User::STATUS, UserStatus::Suspended)
    .where_(User::LOGIN_COUNT.eq(0))
    .without_lifecycle() // optional fast path; skips lifecycle hooks/events
    .execute(&*db)
    .await?;
```

### Instance Method

```rust
let updated = user.update()
    .set(User::NAME, "New Name")
    .save(&*db)
    .await?;
```

### Expression Updates

```rust
User::model_update()
    .set_expr(User::LOGIN_COUNT, Expr::column(User::LOGIN_COUNT.column_ref()) + Expr::value(1))
    .where_(User::ID.eq(user_id))
    .execute(&*db)
    .await?;
```

### Set NULL

```rust
user.update()
    .set_null(User::NICKNAME)
    .save(&*db)
    .await?;
```

---

## Deleting

```rust
// Soft delete (sets deleted_at)
user.delete().execute(&*db).await?;

// Permanent delete
user.force_delete().execute(&*db).await?;

// Bulk soft delete
User::model_delete()
    .where_(User::STATUS.eq(UserStatus::Banned))
    .execute(&*db)
    .await?;

// Restore soft-deleted
User::model_restore()
    .where_(User::ID.eq(user_id))
    .execute(&*db)
    .await?;
```

---

## Relations

### Defining Relations

```rust
#[derive(Model)]
#[foundry(table = "users")]
struct User {
    id: ModelId<Self>,
    name: String,
    orders: Loaded<Vec<Order>>,         // has_many
    profile: Loaded<Option<Profile>>,   // has_one
    order_count: Loaded<i64>,           // aggregate
}

#[derive(Model)]
#[foundry(table = "orders")]
struct Order {
    id: ModelId<Self>,
    user_id: ModelId<User>,
    total: i64,
    author: Loaded<Option<User>>,       // belongs_to
    items: Loaded<Vec<OrderItem>>,      // has_many (nested)
}

impl User {
    fn orders() -> RelationDef<Self, Order> {
        has_many(Self::ID, Order::USER_ID, |u| u.id, |u, orders| u.orders = Loaded::new(orders))
    }

    fn profile() -> RelationDef<Self, Profile> {
        has_one(Self::ID, Profile::USER_ID, |u| u.id, |u, profile| u.profile = Loaded::new(profile))
    }

    fn order_count() -> RelationAggregateDef<Self, i64> {
        Self::orders().count(|u, count| u.order_count = Loaded::new(count))
    }

    fn order_total() -> RelationAggregateDef<Self, Option<i64>> {
        Self::orders().sum(Order::TOTAL, |u, total| u.order_total = Loaded::new(total))
    }
}

impl Order {
    fn author() -> RelationDef<Self, User> {
        belongs_to(Self::USER_ID, User::ID, |o| Some(o.user_id), |o, user| o.author = Loaded::new(user))
    }

    fn items() -> RelationDef<Self, OrderItem> {
        has_many(Self::ID, OrderItem::ORDER_ID, |o| o.id, |o, items| o.items = Loaded::new(items))
    }
}
```

### Eager Loading

```rust
// Load relations
let users = User::model_query()
    .with(User::orders())
    .with(User::profile())
    .with_aggregate(User::order_count())
    .all(&*db).await?;

// Nested eager loading (unlimited depth)
let users = User::model_query()
    .with(User::orders()
        .with(Order::items())
        .with(Order::author()))
    .all(&*db).await?;

// Filtered relations
let users = User::model_query()
    .with(User::orders().where_(Order::TOTAL.gt(100)))
    .all(&*db).await?;

// Eager-load model extensions only when the response needs them
let users = User::model_query()
    .with_attachments("avatar")
    .with(User::orders()
        .with_attachments("invoice")
        .with_translated_field("summary"))
    .all(&*db).await?;
```

`with_attachments(...)`, `with_translated_field(...)`, `with_translations_for(...)`, and `with_all_translations()` preload Foundry extension data into the active model-extension cache. HTTP requests are scoped automatically. In CLI jobs, workers, and tests, wrap the full load-and-access flow in `app.with_model_batching(...)` when you want eager extension data or lazy batch safety outside HTTP.

### Many-to-Many

```rust
impl User {
    fn roles() -> ManyToManyDef<Self, Role, ()> {
        many_to_many(
            Self::ID, "user_roles", "user_id", "role_id", Role::ID,
            |u| u.id, |u, roles| u.roles = Loaded::new(roles),
        )
    }
}

let users = User::model_query()
    .with_many_to_many(User::roles())
    .all(&*db).await?;

let users = User::model_query()
    .with_many_to_many(User::roles().with_translated_field("label"))
    .all(&*db).await?;
```

### Relation Aggregates

```rust
// Count, sum, avg, min, max — computed via subquery, no eager load needed
let users = User::model_query()
    .with_aggregate(User::order_count())     // i64
    .with_aggregate(User::order_total())     // Option<i64>
    .all(&*db).await?;

for user in &users {
    println!("{}: {} orders, ${} total",
        user.name,
        user.order_count.as_ref().unwrap_or(&0),
        user.order_total.as_ref().flatten().unwrap_or(&0));
}
```

### where_has

Filter parent by related records:

```rust
// Users who have at least one completed order
let users = User::model_query()
    .where_has(User::orders(), |q| q.where_(Order::STATUS.eq("completed")))
    .all(&*db).await?;

// Users with the "admin" role (many-to-many)
let admins = User::model_query()
    .where_has_many_to_many(User::roles(), |q| q.where_(Role::NAME.eq("admin")))
    .all(&*db).await?;
```

---

## Lifecycle Hooks

### Write Mutators

Transform values automatically on create/update:

```rust
#[derive(Model)]
#[foundry(table = "users", lifecycle = UserLifecycle)]
struct User {
    id: ModelId<Self>,
    #[foundry(write_mutator = "hash_password")]
    password: String,
}

impl User {
    async fn hash_password(ctx: &ModelHookContext<'_>, value: String) -> Result<String> {
        ctx.app().hash()?.hash(&value)
    }
}
```

Now `User::model_create().set(User::PASSWORD, "plaintext")` automatically hashes before insert.
Optional fields work the same way because Foundry preserves the column type for `NULL` values:

```rust
#[derive(Model)]
#[foundry(table = "users")]
struct User {
    id: ModelId<Self>,
    #[foundry(write_mutator = "normalize_username")]
    username: Option<String>,
}

impl User {
    async fn normalize_username(
        _ctx: &ModelHookContext<'_>,
        value: Option<String>,
    ) -> Result<Option<String>> {
        Ok(value.map(|username| username.trim().to_lowercase()))
    }
}
```

### Lifecycle Trait

```rust
struct UserLifecycle;

#[async_trait]
impl ModelLifecycle<User> for UserLifecycle {
    async fn creating(ctx: &ModelHookContext<'_>, draft: &mut CreateDraft<User>) -> Result<()> {
        // Set defaults, validate business rules
        Ok(())
    }

    async fn created(ctx: &ModelHookContext<'_>, user: &User, _record: &DbRecord) -> Result<()> {
        // Send welcome email, dispatch event
        ctx.dispatch(UserCreatedEvent { user_id: user.id.to_string() }).await?;
        Ok(())
    }

    async fn updating(ctx: &ModelHookContext<'_>, current: &User, draft: &mut UpdateDraft<User>) -> Result<()> {
        // Audit changes
        Ok(())
    }

    async fn deleting(ctx: &ModelHookContext<'_>, user: &User, _record: &DbRecord) -> Result<()> {
        // Cascade cleanup
        Ok(())
    }
}
```

### Hook Context

```rust
ctx.app()          // → &AppContext (full framework access)
ctx.database()     // → &DatabaseManager
ctx.transaction()  // → &DatabaseTransaction (the active transaction)
ctx.actor()        // → Option<&Actor> (who triggered this write)
ctx.executor()     // → &dyn QueryExecutor
ctx.dispatch(event).await?  // dispatch a domain event
```

## Built-in Audit Logging

Foundry can write one audit row per create, update, soft delete, restore, and hard delete. Audit
rows are written inside the same database transaction as the model change, but only for HTTP
requests that resolve to an explicit audit area.

Mark the route tree that should produce audit rows:

```rust
r.scope("/admin", |admin| {
    admin
        .name_prefix("admin")
        .guard(Guard::Admin)
        .audit_area("admin");

    admin.post("/users", "store", create_admin_user, |_| {});
    Ok(())
})?;
```

Control auditing per model and per field:

```rust
#[derive(Model)]
#[foundry(table = "admins")]
struct Admin {
    id: ModelId<Self>,
    email: String,
    #[foundry(audit_exclude)]
    password_hash: String,
    created_at: DateTime,
    updated_at: DateTime,
}

#[derive(Model)]
#[foundry(table = "cache_entries", audit = false)]
struct CacheEntry {
    id: ModelId<Self>,
    key: String,
    value: String,
}
```

Common credential-like column names are redacted by default in audit JSON:

```toml
[audit]
redact_sensitive_fields = true
sensitive_fields = ["password", "password_hash", "secret", "api_key", "token", "refresh_token"]
```

Redacted fields remain present as `"[redacted]"` so reviewers can see that a value exists or
changed without storing the value. Use `#[foundry(audit_exclude)]` when a field should be omitted
entirely, extend `audit.sensitive_fields` for project-specific credential names, or set
`redact_sensitive_fields = false` to return to explicit model-only exclusions.

Query audit rows through the built-in `AuditLog` model:

```rust
let logs = AuditLog::query()
    .where_(AuditLog::SUBJECT_TABLE.eq("admins"))
    .where_(AuditLog::AREA.eq(Some("admin".to_string())))
    .order_by(AuditLog::CREATED_AT.desc())
    .limit(50)
    .all(&*db)
    .await?;
```

Payload shape is stable:

- `event_type`: `created`, `updated`, `soft_deleted`, `restored`, or `deleted`
- `area`: the resolved route/scope audit area, such as `admin`
- `before_data`: full row snapshot before destructive changes
- `after_data`: full row snapshot after create/update/restore/soft delete
- `changes`: dirty-only JSON payload for updates, soft deletes, and restores

Unmarked routes, routes under `audit_disabled()`, and non-HTTP writes from jobs, scheduler tasks,
or CLI commands do not produce audit rows by default.

The framework `AuditLog` model is excluded from auditing automatically to avoid recursion.

---

## Projections

For queries that don't map 1:1 to a model (aggregates, joins, CTEs):

```rust
#[derive(Clone, Projection)]
struct MonthlySales {
    #[foundry(source = "month")]
    month: String,
    #[foundry(source = "total_revenue")]
    total_revenue: f64,
    #[foundry(source = "order_count")]
    order_count: i64,
}

let report = ProjectionQuery::<MonthlySales>::table("orders")
    .select_field(MonthlySales::MONTH, Expr::function("to_char", vec![
        Expr::column(ColumnRef::unqualified("created_at")),
        Expr::value("YYYY-MM"),
    ]))
    .select_field(MonthlySales::TOTAL_REVENUE, Expr::aggregate(AggregateFn::Sum,
        Expr::column(ColumnRef::unqualified("total"))))
    .select_field(MonthlySales::ORDER_COUNT, Expr::aggregate(AggregateFn::Count,
        Expr::column(ColumnRef::unqualified("id"))))
    .group_by(Expr::function("to_char", vec![
        Expr::column(ColumnRef::unqualified("created_at")),
        Expr::value("YYYY-MM"),
    ]))
    .order_by(OrderBy::new(Expr::value("month"), OrderDirection::Desc))
    .all(&*db).await?;
```

Projections support all query features: joins, CTEs, UNION, window functions, pagination, streaming.

---

## Transactions

### Basic

```rust
let mut tx = app.begin_transaction().await?;

let user = User::model_create()
    .set(User::EMAIL, "alice@example.com")
    .save(&tx)
    .await?;

Order::model_create()
    .set(Order::USER_ID, user.id)
    .set(Order::TOTAL, 100)
    .execute(&tx)
    .await?;

tx.commit().await?;  // or tx.rollback().await?
```

### With After-Commit Callbacks

```rust
let mut tx = app.begin_transaction().await?;

// ... create order ...

tx.dispatch_after_commit(SendOrderConfirmation { order_id: order.id.to_string() });
tx.notify_after_commit(&user, &OrderPlacedNotification { /* ... */ });
tx.after_commit(|app| async move {
    // custom cleanup
    Ok(())
});

tx.commit().await?;
// All callbacks run only after successful commit
```

### Model Events and Commit Timing

Model lifecycle hooks run inside the active write transaction. Use `creating`, `updating`, and
`deleting` to validate or mutate data before it is committed; returning an error from these hooks or
their matching `ModelCreatingEvent`/`ModelUpdatingEvent`/`ModelDeletingEvent` listeners aborts the
write.

Foundry defers the framework post-write events until commit succeeds:
`ModelCreatedEvent`, `ModelUpdatedEvent`, and `ModelDeletedEvent` are dispatched after the row is
committed. This makes them safe for onboarding jobs, dependent FK writes, notifications, and other
listeners that need the committed row to be visible from a normal database connection. If one of
these post-commit listeners fails, the committed model write is kept and Foundry logs the listener
failure.

---

## Raw SQL

For queries that can't be expressed with the builders:

```rust
let db = app.database()?;

// Query (returns rows)
let rows = db.raw_query(
    "SELECT id, email FROM users WHERE status = $1 LIMIT $2",
    &[DbValue::Text("active".into()), DbValue::Int64(10)],
).await?;

for row in &rows {
    let email = row.text("email");
    println!("{email}");
}

// Execute (returns affected count)
let affected = db.raw_execute(
    "UPDATE users SET login_count = login_count + 1 WHERE id = $1",
    &[DbValue::Uuid(user_id)],
).await?;
```

---

## Migrations

### Creating a Migration

```bash
cargo run -- make:migration create_posts
```

Creates `database/migrations/YYYYMMDDHHMM_create_posts.rs`:

```rust
use foundry::prelude::*;

pub struct Migration;

#[async_trait]
impl MigrationFile for Migration {
    async fn up(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute(
            "CREATE TABLE posts (
                id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                title TEXT NOT NULL,
                body TEXT NOT NULL,
                published BOOLEAN NOT NULL DEFAULT false,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ
            )",
            &[],
        ).await?;
        Ok(())
    }

    async fn down(ctx: &MigrationContext<'_>) -> Result<()> {
        ctx.raw_execute("DROP TABLE IF EXISTS posts", &[]).await?;
        Ok(())
    }
}
```

### Running Migrations

```bash
cargo run -- db:migrate                       # run pending
cargo run -- db:migrate --lock-timeout-ms 0   # wait forever for migration lock (default)
cargo run -- db:migrate:status                # show status
cargo run -- db:migrate:status --json         # machine-readable status/drift report
cargo run -- db:rollback                      # rollback last batch
PROCESS=cli ./app doctor --deploy --json      # runtime deploy preflight from built binary
```

`db:migrate` and `db:rollback` use a Postgres advisory lock keyed by the configured schema and
migration table. The default lock timeout is `0`, meaning wait forever for compatibility with
rolling deploys. Set `database.migration_lock_timeout_ms` or pass `--lock-timeout-ms` when deploy
tooling should fail instead of waiting behind another migration process. If the migration ledger
contains an applied migration that is not registered in the current binary, `db:migrate:status`
reports it and `db:migrate:status --json` includes it under `missing_applied`; migrate/rollback
remain strict and stop until the binary or migration files are corrected.

For source-free servers, run `doctor --deploy --json` from the compiled binary before stopping
services. The command uses the server-managed `.env`, checks migration drift through the same
lifecycle code as `db:migrate:status`, and lets deploy scripts fail before swapping runtime files.

### Seeders

```bash
cargo run -- make:seeder seed_posts
cargo run -- seed:publish                        # publish framework seeders like countries
cargo build                                      # rebuild so published seeders are discovered
cargo run -- db:seed                             # run all seeders
cargo run -- db:seed --id 000000000001_countries_seeder
cargo run -- seed:countries                      # direct built-in countries seed
```

### Build-Time Discovery

Migrations and seeders are discovered at compile time via `build.rs`:

```rust
// build.rs
fn main() -> std::io::Result<()> {
    foundry_build::DatabaseCodegen::new()
        .migration_dir("database/migrations")
        .seeder_dir("database/seeders")
        .generate()
}
```

Register in your ServiceProvider:

```rust
foundry::register_generated_database!(registrar);
```

---

## Debugging

```rust
// See the compiled SQL
let sql = User::model_query()
    .where_(User::STATUS.eq(UserStatus::Active))
    .to_compiled_sql()?;
println!("{}", sql.sql);
println!("{:?}", sql.bindings);

// EXPLAIN
let plan = User::model_query()
    .where_(User::EMAIL.eq("test@example.com"))
    .explain(&*db).await?;
for line in &plan { println!("{line}"); }

// EXPLAIN ANALYZE
let plan = User::model_query()
    .explain_analyze(&*db).await?;
```

---

## Config

```toml
# config/database.toml
[database]
url = "postgres://foundry:secret@127.0.0.1:5432/foundry"
# read_url = ""                    # read replica
# schema = "public"
# migration_table = "foundry_migrations"
# migration_lock_timeout_ms = 0    # migration advisory-lock wait timeout; 0 waits forever
# migrations_path = "database/migrations"
# seeders_path = "database/seeders"
# min_connections = 1
# max_connections = 10
# acquire_timeout_ms = 5000
# log_queries = false              # log SQL shape to tracing
# log_query_bindings = false       # include binding values when log_queries=true (dev only)
# redact_sql_literals = true       # redact SQL literals/comments in logs and /_foundry/sql
# slow_query_threshold_ms = 500    # log slow queries
# slow_query_retention = 100       # retained slow-query entries for /_foundry/sql; 0 disables retention
# n_plus_one_detection = true      # detect repeated query shapes per HTTP request
# n_plus_one_min_repeats = 10      # minimum repeats before retaining a suspect
# n_plus_one_retention = 100       # retained N+1 suspect entries

[database.models]
# timestamps_default = true        # auto-manage created_at/updated_at
# soft_deletes_default = false      # auto-manage deleted_at
```

`GET /_foundry/sql` exposes retained slow queries, a top-slowest ranking, and potential HTTP
N+1 query suspects. N+1 detection groups repeated SQL fingerprints inside a single HTTP
request; jobs and scheduler runs are intentionally excluded to avoid batch-work noise.

SQL observability data is process-local and bounded in memory. It is cleared on restart and does
not require a database migration or cleanup scheduler. SQL shown in logs and
`/_foundry/sql` redacts string/numeric literals and SQL comments by default, so
slow-query and N+1 diagnostics keep query shape without retaining common secret
values. Set `slow_query_threshold_ms = 0` to disable slow-query capture,
`slow_query_retention = 0` to keep slow-query logs out of the dashboard, or
`n_plus_one_detection = false` to avoid per-query HTTP fingerprinting overhead.
Use `log_query_bindings = true` only for short-lived local debugging; binding
values may contain user data or secrets.
