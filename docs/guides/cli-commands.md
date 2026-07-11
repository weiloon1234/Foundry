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
            inv.line("Hello from Foundry!")?;
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

---

## Defining Commands

Every command needs three things: a `CommandId`, a `clap::Command` definition, and an async handler.

### Simple Command (No Arguments)

```rust
const PING: CommandId = CommandId::new("ping");

reg.command(
    PING,
    Command::new("ping").about("Check if the app is alive"),
    |inv| async move {
        inv.line("pong")?;
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

        inv.line(format!("Importing from: {file}"))?;
        if dry_run {
            inv.line("(dry run — no changes will be made)")?;
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

        inv.line(format!("Exporting as {format} to {output}"))?;
        if let Some(limit) = limit {
            inv.line(format!("Limiting to {limit} rows"))?;
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
        inv.line(format!("Deleted {deleted} expired sessions"))?;

        // Prune expired tokens
        let pruned = app.tokens()?.prune(30).await?;
        inv.line(format!("Pruned {pruned} expired tokens"))?;

        // Cache
        app.cache()?.forget("dashboard:stats").await?;

        // Jobs
        app.jobs()?.dispatch(SendCleanupReport {
            deleted_count: deleted,
        }).await?;

        inv.line("Cleanup complete")?;
        Ok(())
    },
)?;
```

### CommandInvocation Methods

```rust
inv.app()       // → &AppContext (database, cache, redis, email, jobs, etc.)
inv.matches()   // → &ArgMatches (clap argument values)
inv.io()        // → &dyn CommandIo (injectable stdin/stdout/stderr boundary)
```

---

## Output, Prompts, and Progress

Write through the invocation instead of calling `println!` directly. The same
handler then works with the real terminal and with captured test I/O:

```rust
inv.write("no trailing newline")?;
inv.line("normal output")?;
inv.error("diagnostic output")?;

let name = inv.prompt("Project name")?;
if !inv.confirm("Continue", false)? {
    inv.line("Cancelled")?;
    return Ok(());
}

let mut progress = inv.progress("Import", rows.len() as u64)?;
for row in rows {
    import(row).await?;
    progress.advance(1)?;
}
progress.finish()?;
```

Prompt helpers are intentionally synchronous because a CLI owns its terminal.
Progress output is deterministic and line-oriented, so it remains readable in
non-interactive logs and exactly assertable in tests.

## Typed Exit Status

Existing `command(...)` handlers keep returning `Result<()>`. Use
`command_with_exit(...)` when a successful command execution needs to return a
specific process-style status without turning it into a framework error:

```rust
reg.command_with_exit(
    CommandId::new("lint"),
    Command::new("lint"),
    |inv| async move {
        let violations = run_lint(inv.app()).await?;
        Ok(if violations == 0 {
            CommandExit::SUCCESS
        } else {
            CommandExit::new(2)
        })
    },
)?;
```

`CliKernel::run_with_args_status(...)` and `run_status()` return that
`CommandExit`. The compatibility `run_with_args(...)` and `run()` methods still
return `Result<()>` and convert a nonzero status into an error.

## Capturing CLI Tests

Inject `CommandIoFake` into a built kernel to capture streams and queue prompt
answers without touching process-global stdin/stdout:

```rust
let io = CommandIoFake::new().with_input("yes");
let status = build_app()
    .build_cli_kernel()
    .await?
    .with_io(io.clone())
    .run_with_args_status(["foundry", "import:users", "users.csv"])
    .await?;

assert!(status.is_success());
io.assert_stdout_contains("Importing from: users.csv")
    .assert_stderr("");
```

Help and version output use the same injected stream, so they can be captured
without redirecting the whole test process.

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
                    inv.line("Plugin is running")?;
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

### Development Process Orchestration

The built-in `dev` command runs selected application runtimes together by
launching the current executable once per process and setting the same
`PROCESS` selector documented in Getting Started:

```bash
# Run HTTP, worker, scheduler, and WebSocket together
PROCESS=cli cargo run -- dev

# Run only selected processes
PROCESS=cli cargo run -- dev http worker

# Retry a failed process at most three times, starting at a 500 ms delay
PROCESS=cli cargo run -- dev --max-restarts 3 --restart-backoff-ms 500
```

Supported child values are `http`, `worker`, `scheduler`, and `websocket`.
Output is prefixed with labels such as `[http]` and `[worker]`. A clean child
exit is not restarted and stops its siblings so the development stack is never
left partially running. A failed child is restarted only when
`--max-restarts <COUNT>` is nonzero; the delay doubles after each attempt and
is capped at 60 seconds. Counts above 100 and initial delays outside
100–60000 ms are rejected, preventing an accidental hot crash loop.

Ctrl+C or SIGTERM stops every child. If a process exhausts its restart limit,
the command stops the remaining processes and exits unsuccessfully. The app's
entry point must dispatch the four `PROCESS` values to their corresponding
kernels; the command deliberately reuses that one selector instead of defining
a second runtime map.

This is process orchestration only. It does not create, generate, download, or
install a starter project.

---

## Built-in Framework Commands

These are available automatically — no registration needed:

| Command | Description |
|---------|-------------|
| `config:publish` | Generate split sample config files, including auth credential lifecycle, lockout, and MFA sections |
| `env:publish` | Generate `.env.example`, including auth credential lifecycle, lockout, and MFA env overrides |
| `key:generate` | Generate 32-byte signing + encryption keys |
| `dev` | Supervise selected current-executable processes for local development |
| `doctor` | Run runtime health checks; accepts `--deploy`, `--json`, and `--strict` for deploy tooling |
| `migrate:publish` | Publish framework migration files, including audit log and MFA tables |
| `seed:publish` | Publish framework seeder files |
| `db:migrate` | Run pending migrations; accepts `--lock-timeout-ms <MS>` |
| `db:migrate:status` | Show migration status; accepts `--json` for deploy tooling |
| `db:rollback` | Rollback last migration batch; accepts `--lock-timeout-ms <MS>` |
| `db:seed` | Run seeders |
| `seed:countries` | Seed 250 countries |
| `make:migration` | Create a migration file |
| `make:seeder` | Create a seeder file |
| `make:model` | Create a model file; accepts `--path <DIR>` and `--table <TABLE>` |
| `make:job` | Create a job file; accepts `--path <DIR>` |
| `make:command` | Create a command file; accepts `--path <DIR>` |
| `make:request` | Create a validated HTTP request DTO file |
| `make:dto` | Create a serializable API DTO file |
| `make:policy` | Create an authorization policy file |
| `make:event` | Create a domain event file |
| `make:listener` | Create an event listener file; requires `--event <TYPE>` |
| `make:notification` | Create a notification file |
| `make:mail` | Create an email message file |
| `make:datatable` | Create a model datatable file; requires `--model <TYPE>` |
| `make:plugin` | Create an in-app plugin component file |
| `make:test` | Create an integration test file |
| `down` | Enter maintenance mode |
| `up` | Exit maintenance mode |
| `routes:list` | List named routes |
| `token:prune` | Manually prune expired/revoked personal access tokens |
| `attachment:orphans` | Audit or delete old attachment storage objects missing from the attachments table |
| `audit:prune` | Delete audit rows older than `audit.retention_days`; accepts `--days <DAYS>` |
| `plugin:list` | List plugins |
| `plugin:install-assets` | Install plugin assets inside the selected target directory |
| `plugin:scaffold` | Run plugin scaffold inside the selected target directory |
| `docs:api` | Generate API surface docs |
| `about` | Show framework version and environment |

`doctor --deploy --json` is designed for runtime-only servers where the deploy package contains
only the compiled binary and built assets. It uses the server's existing `.env`, checks config,
database, runtime backend, cache roundtrip, readiness probes, and migration drift, then exits
non-zero only when a check fails. Warnings, such as an intentionally unconfigured database or a
Redis cache driver without Redis configured, are reported but do not block the command.

Use `doctor --deploy --strict` as a production readiness gate. Strict mode exits non-zero when
any warning is reported, and text output ends with a `Production readiness: ...` verdict.

`make:model`, `make:job`, and `make:command` default to the legacy `src/app/*` directories. Pass
`--path <DIR>` to target split app layouts such as `src/domain/models`, `src/domain/jobs`, or
`src/commands`. Relative paths are resolved from the current working directory; absolute paths are
used as-is.

`make:model` conservatively pluralizes the model name (`Category` becomes
`categories`, `Status` becomes `statuses`). Pass `--table <TABLE>` for an
irregular, legacy, or otherwise deliberate table name; the value must be a
lowercase PostgreSQL identifier.

The additional `make:*` commands generate one repeated application component
at a time; they do not create or install a starter application. Every command
accepts `--name`, `--path`, and `--force`. Listener and datatable generation
also require the related Rust type:

```bash
cargo run -- make:request --name CreateUserRequest
cargo run -- make:listener --name SendWelcomeEmail --event UserCreated
cargo run -- make:datatable --name UsersDatatable --model User
cargo run -- make:plugin --name AuditToolsPlugin
cargo run -- make:test --name UserRegistrationTest
```

Generated files are intentionally conservative: authorization policies deny
by default, listeners and datatables include conventional imports that the app
may need to adjust, and every command prints the required registration or
customization step after writing the file.

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

`audit:prune` refuses to run when the effective retention window is zero. Set
`audit.retention_days` for the normal policy or pass `--days` for an explicit
operator run. No audit rows are removed automatically after upgrading.

`docs:api` writes generated Markdown under the selected output directory and tracks those files in
`.foundry-api-docs-manifest.json`. Regeneration only removes files owned by that manifest plus the
current planned generated files (`index.md`, `root.md`, and `modules/*.md`), so colocated notes or
manual docs are preserved.

Generated-output commands reject symlinked descendants before creating directories, writing files,
or removing manifest-owned files. A caller-selected output root may itself be a symlink, so linked
workspace and split-repository layouts remain supported.

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
