# CLI Commands Guide

Define custom CLI commands with arguments, flags, and full access to the framework.

---

## Quick Start

```rust
const GREET: CommandId = CommandId::new("greet");

fn commands(reg: &mut CommandRegistry) -> Result<()> {
    reg.command(
        GREET,
        Command::new("greet").about("Say hello"),
        |inv| async move {
            println!("Hello from Foundry!");
            Ok(())
        },
    )?;
    Ok(())
}
```

Register and run:

```rust
App::builder()
    .load_env()
    .load_config_dir("config")
    .register_commands(commands)
    .run_cli()?;
```

```bash
cargo run -- greet
# Hello from Foundry!
```

`types:export` writes registered CLI commands to `CommandManifest.ts`, including
each command id, name, descriptions, argument names, argument metadata (`kind`,
`help`, `longHelp`, `required`, `repeatable`, `valueNames`, `valueHint`,
`defaultValues`, and `possibleValues`), positional argument names, non-positional
option names, value-taking option names, flag names, subcommand names, and
counts. Use `CommandManifest`, `CommandIds`, `commandEntries()`,
`commandNames()`, `commandNameOrNull()`, `isCommandArgumentName()`,
`commandArgumentNameOrNull()`,
`isCommandPositionalArgumentName()`, `commandPositionalArgumentNameOrNull()`,
`isCommandOptionName()`, `commandOptionNameOrNull()`,
`isCommandValueOptionName()`, `commandValueOptionNameOrNull()`,
`isCommandFlagName()`, `commandFlagNameOrNull()`,
`isCommandSubcommandName()`, `commandSubcommandNameOrNull()`,
`commandArguments()`, `commandArgumentMetadata()`,
`commandArgumentMetadataForArgumentOrNull()`,
`commandArgumentHelp()`, `commandArgumentValueNames()`,
`commandArgumentDefaultValues()`, `commandArgumentPossibleValues()`,
`commandVisibleArgumentPossibleValueNames()`, `commandRequiredArgumentNames()`,
`commandRepeatableArgumentNames()`, `commandDefaultedArgumentNames()`,
`commandArgumentNamesWithPossibleValues()`,
`commandPositionalArguments()`, `commandOptions()`, `commandOptionSwitches()`,
`commandOptionTokens()`, `commandPreferredOptionTokenOrNull()`,
`commandValueOptions()`, `commandFlags()`, `commandSubcommands()`,
`commandNamesWithArgument()`, `commandNamesWithPositionalArgument()`,
`commandNamesWithOption()`, `commandNamesWithValueOption()`,
`commandNamesWithFlag()`, `commandNamesWithSubcommand()`,
`commandsWithArguments()`, `commandsWithPositionalArguments()`,
`commandsWithOptions()`, `commandsWithValueOptions()`, `commandsWithFlags()`,
and `commandsWithSubcommands()` in admin or dev tooling instead of copying
command strings or scraping Clap output. Nullable first selectors such as
`commandFirstEntryOrNull()`, `firstCommandWithArgumentOrNull(...)`, and
`firstCommandWithSubcommandsOrNull()` keep empty command groups explicit without
local `... ?? null` wrappers. Generated command constants are frozen at
runtime, while command selector helpers return cloned command entries plus
argument metadata/default-value/possible-value arrays, positional argument,
option, option-switch, value-option, flag, and subcommand arrays for local
dev-tool annotations.

---

## Defining Commands

Every command needs three things: a `CommandId`, a Clap `Command` definition
from `foundry::prelude::*`, and an async handler.

### Simple Command (No Arguments)

```rust
const PING: CommandId = CommandId::new("ping");

reg.command(
    PING,
    Command::new("ping").about("Check if the app is alive"),
    |_inv| async move {
        println!("pong");
        Ok(())
    },
)?;
```

```bash
cargo run -- ping
```

### Command with Arguments

```rust
const IMPORT: CommandId = CommandId::new("import:users");

reg.command(
    IMPORT,
    Command::new("import:users")
        .about("Import users from a CSV file")
        .arg(Arg::new("file")
            .required(true)
            .help("Path to CSV file"))
        .arg(Arg::new("dry-run")
            .long("dry-run")
            .action(ArgAction::SetTrue)
            .help("Preview without writing to database")),
    |inv| async move {
        let file = inv.matches().get_one::<String>("file").unwrap();
        let dry_run = inv.matches().get_flag("dry-run");

        println!("Importing from: {file}");
        if dry_run {
            println!("(dry run — no changes will be made)");
        }

        // ... import logic ...

        Ok(())
    },
)?;
```

```bash
cargo run -- import:users users.csv
cargo run -- import:users users.csv --dry-run
```

### Command with Optional Arguments and Defaults

```rust
const EXPORT: CommandId = CommandId::new("export:orders");

reg.command(
    EXPORT,
    Command::new("export:orders")
        .about("Export orders to file")
        .arg(Arg::new("format")
            .long("format")
            .default_value("csv")
            .help("Output format: csv or json"))
        .arg(Arg::new("output")
            .long("output")
            .short('o')
            .default_value("exports/orders")
            .help("Output directory"))
        .arg(Arg::new("limit")
            .long("limit")
            .value_parser(clap::value_parser!(u64))
            .help("Maximum rows to export")),
    |inv| async move {
        let format = inv.matches().get_one::<String>("format").unwrap();
        let output = inv.matches().get_one::<String>("output").unwrap();
        let limit = inv.matches().get_one::<u64>("limit");

        println!("Exporting as {format} to {output}");
        if let Some(limit) = limit {
            println!("Limiting to {limit} rows");
        }

        Ok(())
    },
)?;
```

```bash
cargo run -- export:orders
cargo run -- export:orders --format json --output /tmp/orders --limit 1000
```

---

## Accessing Framework Services

The `CommandInvocation` gives you full access to the app:

```rust
const CLEANUP: CommandId = CommandId::new("cleanup:expired");

reg.command(
    CLEANUP,
    Command::new("cleanup:expired").about("Remove expired records"),
    |inv| async move {
        let app = inv.app();

        // Database
        let db = app.database()?;
        let deleted = db.raw_execute(
            "DELETE FROM sessions WHERE expires_at < NOW()",
            &[],
        ).await?;
        println!("Deleted {deleted} expired sessions");

        // Prune expired tokens
        let pruned = app.tokens()?.prune(30).await?;
        println!("Pruned {pruned} expired tokens");

        // Cache
        app.cache()?.forget("dashboard:stats").await?;

        // Jobs
        app.jobs()?.dispatch(SendCleanupReport {
            deleted_count: deleted,
        })?;

        println!("Cleanup complete");
        Ok(())
    },
)?;
```

### CommandInvocation Methods

```rust
inv.app()       // → &AppContext (database, cache, redis, email, jobs, etc.)
inv.matches()   // → &ArgMatches (clap argument values)
```

---

## Registering Commands

### In AppBuilder

```rust
App::builder()
    .register_commands(commands)
    .run_cli()?;
```

### In a ServiceProvider (for framework-level commands)

Not common for app commands — use `register_commands` on AppBuilder instead.

### In a Plugin

```rust
impl Plugin for MyPlugin {
    fn register(&self, r: &mut PluginRegistrar) -> Result<()> {
        r.register_commands(|reg| {
            reg.command(
                CommandId::new("my-plugin:status"),
                Command::new("my-plugin:status").about("Show plugin status"),
                |inv| async move {
                    println!("Plugin is running");
                    Ok(())
                },
            )?;
            Ok(())
        });
        Ok(())
    }
}
```

---

## Running Commands

```bash
# Run a specific command
cargo run -- greet

# Run with arguments
cargo run -- import:users data.csv --dry-run

# List all available commands (built-in)
cargo run -- --help
```

The `--` separates cargo arguments from your app's arguments.

### Running the CLI Kernel

The CLI kernel runs the command matching `std::env::args()` and exits:

```rust
// In main.rs — detect CLI mode
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // If first arg looks like a command, run CLI
    if args.len() > 1 && !args[1].starts_with('-') {
        return bootstrap::cli().run_cli();
    }

    // Otherwise run HTTP server
    bootstrap::http().run_http()
}
```

Or use the `PROCESS` env var pattern from the [Getting Started guide](getting-started.md).

---

## Built-in Framework Commands

These are available automatically — no registration needed:

| Command | Description |
|---------|-------------|
| `config:publish` | Generate split sample config files, including auth credential lifecycle, lockout, and MFA sections |
| `env:publish` | Generate `.env.example`, including auth credential lifecycle, lockout, and MFA env overrides |
| `key:generate` | Generate 32-byte signing + encryption keys |
| `doctor` | Run runtime health checks; accepts `--deploy`, `--json`, and `--strict` for deploy tooling |
| `migrate:publish` | Publish framework migration files, including audit log and MFA tables |
| `seed:publish` | Publish framework seeder files |
| `db:migrate` | Run pending migrations; accepts `--lock-timeout-ms <MS>` |
| `db:migrate:status` | Show migration status; accepts `--json` for deploy tooling |
| `db:rollback` | Rollback last migration batch; accepts `--lock-timeout-ms <MS>` |
| `db:seed` | Run seeders |
| `seed:countries` | Seed 250 countries |
| `make:migration` | Create a raw SQL migration file |
| `make:seeder` | Create a seeder file |
| `make:ids` | Create a typed app IDs file; accepts `--path <DIR>` |
| `make:model` | Create a typed model file; accepts `--path <DIR>` |
| `make:request` | Create a typed request DTO file; accepts `--path <DIR>` |
| `make:response` | Create a typed response DTO file; accepts `--path <DIR>` |
| `make:guard` | Create an auth guard authenticator file; accepts `--path <DIR>` |
| `make:policy` | Create an auth policy file; accepts `--path <DIR>` |
| `make:job` | Create a job file; accepts `--path <DIR>` |
| `make:event` | Create a typed event file; accepts `--path <DIR>` |
| `make:notification` | Create a typed notification file; accepts `--path <DIR>` |
| `make:command` | Create a command file; accepts `--path <DIR>` |
| `down` | Enter maintenance mode |
| `up` | Exit maintenance mode |
| `routes:list` | List named routes |
| `token:prune` | Manually prune expired/revoked personal access tokens |
| `attachment:orphans` | Audit or delete old attachment storage objects missing from the attachments table |
| `plugin:list` | List plugins |
| `plugin:install-assets` | Install plugin assets inside the selected target directory |
| `plugin:scaffold` | Run plugin scaffold inside the selected target directory |
| `docs:api` | Generate API surface docs |
| `about` | Show framework version and environment |

`doctor --deploy --json` is designed for runtime-only servers where the deploy package contains
only the compiled binary and built assets. It uses the server's existing `.env`, checks config,
database, runtime backend, cache roundtrip, readiness probes, and migration drift, then exits
non-zero only when a check fails. Warnings, such as an intentionally unconfigured database or a
Redis cache driver without Redis configured, are reported but do not block the command. Pending
migrations are reported as a warning so deploy tooling can inspect a package before applying
migrations.

Use `doctor --deploy --strict` as a production readiness gate. Strict mode exits non-zero when
any warning is reported, including pending migrations, and text output ends with a
`Production readiness: ...` verdict.

`make:ids`, `make:model`, `make:request`, `make:response`, `make:guard`,
`make:policy`, `make:job`, `make:event`, `make:notification`, and
`make:command` default to the legacy `src/app/*` directories. Pass
`--path <DIR>` to target split app layouts such as `src/domain`,
`src/domain/models`, `src/http/requests`, `src/http/responses`, `src/domain/guards`,
`src/domain/policies`, `src/domain/jobs`, `src/domain/events`,
`src/domain/notifications`, or `src/commands`. Relative paths are resolved from
the current working directory; absolute paths are used as-is.

`make:model` scaffolds `serde::Serialize`, `foundry::ts_rs::TS`, `ApiSchema`,
and `Model`, so newly generated models are immediately eligible for
`types:export` and OpenAPI route docs when used as response contracts. The
scaffold expects the same `serde` dependency used by generated jobs, but does
not require a direct `ts-rs` dependency. `make:migration` remains a raw SQL
scaffold; edit the `ctx.raw_execute(...)` calls directly. New migration,
seeder, guard, policy, and job scaffolds use `#[foundry::async_trait]`, so they
do not require a direct `async-trait` dependency.

`make:ids` scaffolds a starter `ids.rs` with `FoundryId` guard and route enums
plus an `AppEnum` ability enum backed by `PermissionId`, so app-owned semantic
ids live in one backend module and ability values can be exported to TypeScript.

`make:request` scaffolds `serde::Deserialize`, `foundry::ts_rs::TS`,
`ApiSchema`, and `Validate`, so newly generated request DTOs are immediately
usable with `JsonValidated<T>` / `Validated<T>` and eligible for generated
TypeScript route helpers and OpenAPI validation metadata.

`make:response` scaffolds `serde::Serialize`, `foundry::ts_rs::TS`, and
`ApiSchema`, so newly generated response DTOs are immediately usable with
`route.response::<T>(status)`, `types:export`, and OpenAPI route docs without a
direct `ts-rs` dependency.

`make:guard` scaffolds a backend-owned `GuardId`, a `register(...)` helper, and
a `BearerAuthenticator` implementation that rejects by default until you fill in
`authenticate(...)`. A trailing `Guard` suffix is removed from the generated id,
so `ApiGuard` registers `GuardId::new("api")`. After registration,
`types:export` includes the guard id in `AuthManifest.ts` / `AuthGuardIds`.

`make:policy` scaffolds a backend-owned `PolicyId`, a `register(...)` helper,
and a `Policy` implementation that denies by default until you fill in
`evaluate(...)`. After registration, `types:export` includes the policy id in
`AuthManifest.ts` / `AuthPolicyIds` without copying the id into frontend code.

`make:job` scaffolds `serde::Serialize`, `serde::Deserialize`,
`foundry::ts_rs::TS`, and `foundry::TS`, so the job payload type is exportable
by `types:export`. Register `TsJobPayload` after registering the job when
dashboards or tooling need `JobPayloadMap` to point at that payload type.

`make:event` scaffolds `serde::Serialize`, `serde::Deserialize`,
`foundry::ts_rs::TS`, `foundry::TS`, `ApiSchema`, `Event`, and a
`TsEventPayload` registration, so the event payload is immediately exportable
through `EventManifest.ts`. The generated event id uses dot notation, for
example `OrderPlaced` becomes `order.placed`.

`make:notification` scaffolds `serde::Serialize`, `serde::Deserialize`,
`foundry::ts_rs::TS`, `foundry::TS`, `ApiSchema`, `Notification`, and a
`TsNotification` registration. It defaults to database and broadcast channels,
serializes the notification struct as both payloads, and uses dot notation for
the notification type, for example `OrderShipped` becomes `order.shipped`.

`make:command` uses the prelude-exported `Command`, so generated commands do
not need a direct `clap` dependency. Existing generated command files can either
keep `clap` in the app's `Cargo.toml` or replace `clap::Command::new(...)` with
`Command::new(...)` when they already import `foundry::prelude::*`.

`config:publish` writes grouped `config/*.toml` files by default. Foundry loads those files in
lexical filename order, merges them in memory, then applies `.env` overrides. Use
`config:publish --single-file` only when you need the legacy `config/foundry.toml` layout.

`config:publish` includes the current framework-owned auth sections, including token pruning,
per-guard token TTL examples, `[auth.password_resets]`, `[auth.email_verification]`,
`[auth.lockout]`, and `[auth.mfa]`. `env:publish` writes the matching `AUTH__TOKENS__*`,
`AUTH__PASSWORD_RESETS__*`, `AUTH__EMAIL_VERIFICATION__*`, `AUTH__LOCKOUT__*`,
`AUTH__MFA__*`, and `AUTH__MFA__REQUIRED_ROLES__<GUARD>` entries. Built-in audit logging is
activated in code via `audit_area(...)`; `[audit]` / `AUDIT__*` config only controls default
credential-like field redaction in audit payloads. Error reporters are
registered in code with `AppBuilder::register_error_reporter*()`, so they are intentionally not
part of the generated config files either.

`attachment:orphans` checks list-capable storage disks against `attachments.disk/path`. It accepts
`--json`, `--disk <name>`, `--limit <n>`, `--older-than-seconds <n>`, and `--delete`. Deletion is
guarded by `storage.attachment_orphan_delete_enabled = true`; the default is audit/log only.

`docs:api` writes generated Markdown under the selected output directory and tracks those files in
`.foundry-api-docs-manifest.json`. Regeneration only removes files owned by that manifest plus the
current planned generated files (`index.md`, `root.md`, and `modules/*.md`), so colocated notes or
manual docs are preserved.

---

## Command ID Naming Convention

Use colon-separated namespaces:

```rust
// Domain commands
CommandId::new("import:users")
CommandId::new("import:products")
CommandId::new("export:orders")
CommandId::new("cleanup:expired")

// Admin commands
CommandId::new("admin:create")
CommandId::new("admin:reset-password")

// Plugin commands
CommandId::new("my-plugin:sync")
```

The command ID must match the `clap::Command` name exactly:

```rust
reg.command(
    CommandId::new("import:users"),         // ← these must match
    Command::new("import:users"),           // ← these must match
    handler,
)?;
```

Duplicate command IDs are rejected at registration time with a clear error.

---

## Clap Features

Foundry uses [clap](https://docs.rs/clap) for argument parsing. All clap features work:

### Subcommands (not recommended — use flat namespace instead)

Foundry commands are flat (`import:users`, not `import users`). This is intentional — flat namespaces are easier to discover via `--help` and don't conflict with the framework's command routing.

### Value Parsers

```rust
Arg::new("count")
    .value_parser(clap::value_parser!(u64))  // parsed as u64
    .help("Number of items")

// Read as:
let count: u64 = *inv.matches().get_one::<u64>("count").unwrap();
```

### Multiple Values

```rust
Arg::new("tags")
    .long("tag")
    .action(ArgAction::Append)  // can pass multiple times
    .help("Tags to apply")

// cargo run -- my-command --tag foo --tag bar
let tags: Vec<&String> = inv.matches().get_many::<String>("tags")
    .map(|v| v.collect())
    .unwrap_or_default();
```

### Boolean Flags

```rust
Arg::new("verbose")
    .long("verbose")
    .short('v')
    .action(ArgAction::SetTrue)

let verbose = inv.matches().get_flag("verbose");
```
