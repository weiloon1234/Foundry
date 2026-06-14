# Foundry App Builder + Service Provider + Container Lifecycle Blueprint

> **Status:** ✅ Largely implemented — reviewed 2026-04-11
>
> Based on current Foundry implementation progress, with rough pass review and stabilization suggestions.
>
> This document is **framework-level**, not consumer-app scaffold-level.

---

# Implementation Status (2026-04-11 Review)

| Part | Status | Notes |
|------|--------|-------|
| App Builder | ✅ Done | All builder methods implemented, internal phases match spec, 5 runtimes + async variants |
| Service Provider | ✅ Done | Two-phase lifecycle (`register` + `boot`), register is side-effect light, boot can resolve services |
| Container | ✅ Done | Singleton + factory bindings, strict no-overwrite (intentional design for predictability) |
| Lifecycle Ordering | ✅ Done | Boot order matches spec, plugin providers run before app providers |
| Runtime Kernel Contracts | ✅ Done | All 5 kernels receive fully built `BootArtifacts`, services ready |
| Gap Review | ⚠️ See below | Minor documentation items only |

---

# Status Context

This blueprint is written against the **current Foundry reality**, not a greenfield fantasy design.

Meaning:

- Foundry already has a working `App::builder()` surface
- Foundry already has provider registration
- Foundry already has a container / DI model
- Foundry already has multiple kernels (HTTP, CLI, Scheduler, WebSocket, Worker)
- Foundry already has many modules implemented beyond the original blueprint
- some behavior is already stable, while some areas still need contract tightening

So this document is:

- partly documentation of current direction
- partly rough gap review
- partly recommendation for what should be frozen next

---

# Objective

Lock down the **top-level framework contract** that every consumer app will depend on:

1. `App::builder()` lifecycle
2. service provider lifecycle and semantics
3. container registration / resolution / override rules
4. boot order rules across framework modules and runtimes

If this layer is unstable, all future consumer app scaffolds will drift.

---

# Why This Matters

Foundry is already far enough along that:

- HTTP exists
- validation exists
- jobs exist
- i18n exists
- logging exists
- storage exists
- email exists
- plugin exists
- ORM is substantial

That means the app-builder/provider/container contract is no longer theoretical.

It is the **real framework spine**.

---

# Current Reality Summary

From current framework progress, these facts are already true:

## App Builder
- `App::builder()` is the main public bootstrap entrypoint
- builder already supports:
  - env loading
  - config directory loading
  - provider registration
  - route registration
  - command registration
  - schedule registration
  - validation rule registration
  - middleware registration
  - runtime execution methods like `run_http()`

## Service Providers
- provider registration is already a first-class concept
- providers are already central to app bootstrapping
- plugin/module-style extension points already exist

## Container
- Foundry already has DI / container behavior in `foundation`
- service resolution is already important across modules
- error conversion and framework-wide service access already depend on container consistency

## Kernels
- multiple runtime kernels already exist:
  - HTTP
  - CLI
  - Scheduler
  - WebSocket
  - Worker
- these kernels already depend on consistent boot lifecycle

---

# Core Design Principle

## Consumer app should only do bootstrap + registration

Meaning consumer apps should not manually wire:

- logger internals
- i18n internals
- scheduler internals
- queue internals
- middleware stack internals
- container internals

Instead, consumer apps should only declare intent:

```rust
App::builder()
    .load_env()
    .load_config_dir("config")
    .register_provider(AppServiceProvider)
    .register_routes(app::portals::router)
    .register_commands(app::commands::register)
    .register_schedule(app::schedules::register)
    .register_validation_rule("mobile", MobileRule)
    .register_middleware(...)
    .run_http()?;
```

This public surface should become stable.

---

# Part 1 — App Builder Blueprint

> **Status: ✅ Done** — Builder methods, internal phases, and runtime methods all implemented.

## Role of App Builder

`App::builder()` is the framework’s main composition root.

It should own:

- config bootstrap
- provider registration queue
- service binding phase
- middleware registration queue
- route/command/schedule registration queue
- final runtime boot

It should **not** directly become a giant service locator.

---

## Recommended Builder Responsibilities

### 1. Environment bootstrap

```rust
.load_env()
```

Loads `.env` once early.

### 2. Config bootstrap

```rust
.load_config_dir("config")
```

Loads typed config and env overlay.

### 3. Framework/service providers

```rust
.register_provider(AppServiceProvider)
```

Queues provider registration.

### 4. Registries

```rust
.register_routes(...)
.register_commands(...)
.register_schedule(...)
.register_validation_rule(...)
.register_middleware(...)
```

These should feed internal registries, not execute immediately.

### 5. Runtime execution

```rust
.run_http()
.run_cli()
.run_worker()
.run_scheduler()
.run_websocket()
```

These methods should trigger final build + boot + kernel handoff.

---

## Suggested App Builder Internal Phases

### Phase A — Collect
Builder collects:
- config sources
- providers
- registries
- middleware configs
- optional framework feature overrides

### Phase B — Build App
Builder creates a concrete `App` instance with:
- config loaded
- container initialized
- base framework services registered
- user providers registered

### Phase C — Boot
Providers and framework modules boot.

### Phase D — Runtime handoff
Selected kernel receives a fully built app.

---

## Important Rule

Builder methods should mostly be **declarative registration**, not immediate execution.

That keeps ordering and lifecycle deterministic.

---

# Part 2 — Service Provider Blueprint

> **Status: ✅ Done** — `ServiceProvider` trait with `register()` and `boot()` phases implemented. Plugin providers run before app providers.

## Role of Service Provider

Providers are the framework-approved unit of bootstrapping.

Providers should be used for:

- service bindings
- module setup
- event listener registration
- command registration if needed
- plugin/module integration
- feature boot logic

They should not become random dumping grounds.

---

## Recommended Provider Trait Shape

Conceptually:

```rust
pub trait ServiceProvider {
    fn register(&self, app: &mut App) -> Result<()>;
    fn boot(&self, app: &App) -> Result<()>;
}
```

Exact signatures may differ, but the semantics should stay clear.

---

## Required Semantics

### `register()`
Use for:
- binding services into the container
- registering internal definitions
- queueing listeners / policies / registries

Should be:
- side-effect light
- dependency-safe
- not dependent on fully booted services where possible

### `boot()`
Use for:
- actions that require resolved services
- final registration into already-built registries
- runtime-ready initialization

Should happen after:
- config loaded
- base container built
- all providers registered

---

## Recommended Provider Rules

### Rule 1
`register()` must not assume every provider is already booted.

### Rule 2
`boot()` may resolve services, because container should now be ready.

### Rule 3
Providers should be deterministic and idempotent where possible.

### Rule 4
Providers should not hide runtime-specific side effects unless clearly intended.

---

## Suggested Provider Lifecycle

```text
collect providers
   ↓
run all register()
   ↓
freeze container registration phase
   ↓
run all boot()
   ↓
hand off to runtime kernel
```

This is the cleanest mental model.

---

# Part 3 — Container Blueprint

> **Status: ✅ Done** — Singleton + factory bindings with strict no-overwrite. Container uses `TypeId` for service identification and `RwLock` for thread safety.

## Role of Container

The container is the framework service registry / resolver.

It should provide:

- singleton bindings
- transient bindings
- resolved service access
- override support
- test-time replacement

It should not become magical global state.

---

## Current Reality

Foundry already depends on consistent service resolution for:

- logging
- i18n
- config
- storage
- email
- jobs
- redis
- ORM-related services
- provider-driven registrations

So container behavior must now be formally defined.

---

## Must Define

### Singleton
One shared instance for app lifetime.

Use for:
- logger
- config
- i18n manager
- storage manager
- email manager
- redis manager

### Transient
New instance per resolution.

Use carefully for lightweight/stateless helpers.

### Scoped / request-aware services
Optional later, but should be planned.

Examples:
- request context helpers
- transaction-scoped services

---

## Recommended Container Rules

### Rule 1
Framework core services register first.

### Rule 2
User providers may add or override services intentionally.

### Rule 3
Overrides should be explicit and predictable.

### Rule 4
Resolution after boot should be stable.

### Rule 5
Container should support test replacement cleanly.

---

## Important Suggestion

If not already formalized, define whether Foundry allows:

- silent overwrite
- guarded overwrite
- named override APIs

My recommendation:

- default to guarded overwrite or explicit replace
- avoid silent accidental replacement

---

# Part 4 — Lifecycle Ordering Blueprint

> **Status: ✅ Done** — Boot order matches this spec. `bootstrap()` in `src/foundation/app.rs` follows: env → config → container → providers register → providers boot → finalize registries → kernel handoff.

## Recommended Global Boot Order

### 1. Build builder state
Collect all registrations.

### 2. Load env
`.env` and process env.

### 3. Load config
Typed config sections.

### 4. Create app/container foundation
Base `App`, base container, base framework service bindings.

### 5. Register framework providers
Core modules bind themselves.

### 6. Register app/plugin providers
App-level and plugin-level bindings.

### 7. Boot framework providers
Now framework modules may resolve services.

### 8. Boot app/plugin providers
Now app may finalize higher-level integration.

### 9. Finalize registries
Routes, commands, schedules, validation rules, middleware.

### 10. Start selected kernel
HTTP / CLI / Worker / WebSocket / Scheduler.

---

## Why this order matters

Without this order, you get bugs like:
- route registration before auth is ready
- logger not ready when kernel starts
- i18n not loaded before validation extracts messages
- plugin assets/routes loaded before plugin services exist

This must be framework law.

---

# Part 5 — Runtime Kernel Contract Implications

> **Status: ✅ Done** — All 5 kernels (HTTP, CLI, Worker, Scheduler, WebSocket) receive fully built `BootArtifacts` with all services ready.

This blueprint is not the full kernel blueprint, but it affects all kernels.

## HTTP kernel expects
- routes finalized
- middleware stack finalized
- logger ready
- i18n ready
- auth providers ready

## CLI kernel expects
- command registry finalized
- config ready
- logger ready
- service container usable

## Worker kernel expects
- jobs infrastructure ready
- queue driver ready
- logger ready
- error handling ready

## Scheduler kernel expects
- schedules finalized
- redis/leadership services ready if used
- logger ready

## WebSocket kernel expects
- channel registry ready
- auth integration ready
- pub/sub services ready

---

# Part 6 — Gap Review (Updated 2026-04-11)

> All original gaps are now resolved or acknowledged as intentional design.

## 1. Builder contract freeze ✅
Builder methods and semantics are frozen. All registration methods are declarative (queue, not execute).

## 2. Provider lifecycle semantics ✅
Explicit two-phase: `register(&mut ServiceRegistrar)` then `boot(&AppContext)`. Register is side-effect light, boot can resolve services.

## 3. Container override rules ✅ (intentional)
Strict no-overwrite is the chosen design. Returning an error on duplicate registration prevents silent accidental replacement. This is intentional for predictability and safety.

## 4. Runtime-specific service readiness guarantees ✅
All kernels receive fully built `BootArtifacts`. Services are guaranteed ready before kernel handoff.

## 5. Plugin/provider interaction boundaries ✅
Plugin providers register and boot before app providers. Framework core services are set up in `bootstrap()` before any providers run.

## 6. Scoped/request-aware services — Deferred
Not needed currently. Axum's State + Extensions model handles per-request state differently than traditional DI scopes. Can be revisited if needed.

These are the likely framework-level areas still worth tightening.

## 1. Builder contract freeze
Even if working now, method semantics should be frozen before scaffolds proliferate.

## 2. Provider lifecycle semantics
Need explicit written contract:
- what belongs in `register()`
- what belongs in `boot()`
- what can resolve services when

## 3. Container override rules
Need clearer formal behavior.

## 4. Runtime-specific service readiness guarantees
Need explicit declaration by kernel.

## 5. Plugin/provider interaction boundaries
Need written rules so plugins do not cause lifecycle drift.

## 6. Testing support integration
Container/provider lifecycle should explicitly support testing overrides and fake services.

---

# Part 7 — Suggested Stable Public Contract

The goal is for consumer apps to rely on something like this long-term:

```rust
use foundry::prelude::*;

fn main() -> Result<()> {
    App::builder()
        .load_env()
        .load_config_dir("config")
        .register_provider(AppServiceProvider)
        .register_routes(app::portals::router)
        .register_commands(app::commands::register)
        .register_schedule(app::schedules::register)
        .register_validation_rule("mobile", MobileRule)
        .register_middleware(...)
        .run_http()?;

    Ok(())
}
```

And for alternate runtimes:

```rust
App::builder()
    .load_env()
    .load_config_dir("config")
    .register_provider(AppServiceProvider)
    .register_commands(app::commands::register)
    .run_cli()?;
```

That surface should become boring and stable.

---

# Part 8 — What Not To Do

## Do not let builder become a service locator API zoo
Keep it focused on registration and runtime selection.

## Do not blur provider `register()` and `boot()`
That creates lifecycle bugs later.

## Do not allow implicit random ordering
Provider ordering and registry finalization must be deterministic.

## Do not let consumer apps wire framework internals manually
That defeats the whole purpose of Foundry.

## Do not freeze scaffold before freezing core contracts
Otherwise every starter app will drift with framework changes.

---

# Part 9 — Recommendation for Next Blueprint After This

The most useful next framework-level blueprint after this is:

## Option A — Runtime Kernel Contracts Blueprint
If you want to formalize HTTP / CLI / Worker / Scheduler / WebSocket readiness and lifecycle.

## Option B — Container Resolution + Override Rules Blueprint
If you want to lock dependency and testability behavior first.

## Option C — Database AST / ORM Core Blueprint
If you want to settle the hardest technical foundation.

My recommendation:

1. finish this builder/provider/container contract
2. then do runtime kernel contracts
3. then do container resolution detail or ORM AST

---

# Final Conclusion

This blueprint is still fully **framework-stage** work.

It exists to stabilize the top-level Foundry contract before consumer scaffold shapes are frozen.

The real rule is:

> Builder defines composition.
> Provider defines modular boot.
> Container defines service truth.
> Kernels consume the fully built app.

And that must become stable before templates multiply.

