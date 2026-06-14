# Plugin System V2 Blueprint (Framework-Level)

## Overview

This document defines the full design of Foundry's **plugin system** — covering what's built, what's missing, and phased improvements to bring it to ecosystem-grade maturity.

Goal:

> Provide a compile-time, dependency-aware plugin system where third-party plugins can register any framework feature (routes, guards, jobs, middleware, etc.) without requiring verbose ServiceProvider wrappers — with conflict detection that prevents silent failures in multi-plugin setups.

---

# Current State

**Status: Core complete — registration gaps and safety nets missing**

### What's Built

- Compile-time plugin registration with `Plugin` trait
- Dependency resolution via topological sort with cycle detection
- SemVer version constraint validation (plugin deps + Foundry compat)
- 3 lifecycle hooks: `manifest()`, `register()`, `boot(app)`
- Config defaults (merge before app config, overridable)
- Asset distribution with install commands + overwrite protection
- Scaffold templating with variable substitution
- CLI commands: `plugin:list`, `plugin:install-assets`, `plugin:scaffold`
- Test coverage: happy paths, fixtures, multi-kernel acceptance

### What's Missing (Priority-Ordered)

| Feature | Priority | Impact |
|---------|----------|--------|
| Direct registration for guards, jobs, events, etc. | **Critical** | Plugin authors forced into verbose 2-step ServiceProvider pattern |
| Middleware registration | **Critical** | Plugins cannot influence HTTP pipeline at all |
| Conflict detection (routes, commands, services) | **High** | Two plugins silently overwrite each other, hard to debug |
| Config namespace convention enforcement | **High** | Plugins compete for same config keys |
| Plugin metadata query API | **Medium** | Cannot inspect plugin capabilities at runtime |
| Shutdown lifecycle hook | **Low** | No cleanup on app stop (Drop usually sufficient) |
| Plugin feature flags | **Low** | Cannot conditionally enable/disable plugin features |

---

# Phase 1: Registration Parity

## Problem

PluginRegistrar can only register 9 things directly. ServiceRegistrar can register 15. This forces plugin authors into a verbose 2-step pattern for common operations:

```rust
// Current: verbose, obscures intent
impl Plugin for MyPlugin {
    fn register(&self, r: &mut PluginRegistrar) -> Result<()> {
        r.register_provider(MyPluginProvider);  // indirection
        Ok(())
    }
}

impl ServiceProvider for MyPluginProvider {
    async fn register(&self, r: &mut ServiceRegistrar) -> Result<()> {
        r.register_guard("my_guard", MyGuard)?;  // actual work
        r.register_job::<MyJob>()?;
        Ok(())
    }
}
```

## Solution

Add convenience methods to PluginRegistrar that match ServiceRegistrar's API. These are stored as type-erased actions and applied during bootstrap — the existing ServiceProvider workaround continues to work, but is no longer required.

### New Methods on PluginRegistrar

```rust
// Auth
fn register_guard<I: Into<GuardId>, G: BearerAuthenticator>(&mut self, id: I, guard: G) -> &mut Self
fn register_policy<I: Into<PolicyId>, P: Policy>(&mut self, id: I, policy: P) -> &mut Self
fn register_authenticatable<M: Authenticatable>(&mut self) -> &mut Self

// Events
fn listen_event<E: Event, L: EventListener<E>>(&mut self, listener: L) -> &mut Self

// Jobs
fn register_job<J: Job>(&mut self) -> &mut Self
fn register_job_middleware<M: JobMiddleware>(&mut self, middleware: M) -> &mut Self

// Notifications
fn register_notification_channel<I: Into<NotificationChannelId>, N: NotificationChannel>(&mut self, id: I, channel: N) -> &mut Self

// Data
fn register_datatable<D: ModelDatatable>(&mut self) -> &mut Self

// Observability
fn register_readiness_check<I: Into<ProbeId>, C: ReadinessCheck>(&mut self, id: I, check: C) -> &mut Self

// Drivers
fn register_storage_driver(&mut self, name: &str, factory: StorageDriverFactory) -> &mut Self
fn register_email_driver(&mut self, name: &str, factory: EmailDriverFactory) -> &mut Self

// HTTP
fn register_middleware(&mut self, config: MiddlewareConfig) -> &mut Self
```

### Internal Design

Type-erased closure pattern — each method captures the generic type at call site and stores a `Box<dyn FnOnce(&ServiceRegistrar) -> Result<()>>`:

```rust
type RegistrarAction = Box<dyn FnOnce(&ServiceRegistrar) -> Result<()> + Send>;

pub struct PluginRegistrar {
    // ... existing fields ...
    middlewares: Vec<MiddlewareConfig>,
    registrar_actions: Vec<RegistrarAction>,
}
```

### Bootstrap Integration

In `bootstrap()`, after plugin providers register but before app providers:

```rust
// 1. Plugin providers register (existing)
for provider in &prepared_plugins.providers {
    provider.register(&mut registrar).await?;
}

// 2. Plugin direct registrations (NEW)
for action in prepared_plugins.registrar_actions {
    action(&registrar)?;
}

// 3. App providers register (existing)
for provider in &providers {
    provider.register(&mut registrar).await?;
}
```

Plugin middlewares merge before app middlewares:

```rust
let mut boot_middlewares = prepared_plugins.middlewares;
boot_middlewares.extend(app_middlewares);
```

### Consumer DX (after)

```rust
// Clean: direct registration, no ServiceProvider wrapper
impl Plugin for MyPlugin {
    fn register(&self, r: &mut PluginRegistrar) -> Result<()> {
        r.register_guard("my_guard", MyGuard);
        r.register_job::<MyJob>();
        r.listen_event::<UserCreated, _>(OnUserCreated);
        r.register_middleware(MiddlewareConfig::from(
            RateLimit::per_minute(100).by_ip()
        ));
        r.register_routes(my_routes);
        Ok(())
    }
}
```

### Precedence Rules

- Plugin registrations run **before** app registrations
- If a plugin and app register the same guard ID, app wins (applied second)
- Plugin middleware runs **before** app middleware in the stack
- Config defaults from plugins are overridden by app config (existing behavior)

---

# Phase 2: Conflict Detection

## Problem

Two plugins can silently overwrite each other's registrations:

| Conflict | Current Behavior |
|----------|------------------|
| Same service type in container | Last wins, no warning |
| Same route path | Undefined (depends on router) |
| Same command ID | Last wins, no warning |
| Same schedule ID | Last wins, no warning |
| Same config key | TOML merge, last writer wins |
| Same asset target path | Error at install time (not registration) |

## Solution

Add validation passes during bootstrap that detect conflicts and produce clear warnings or errors.

### 2.1 Route Path Collision Detection

After collecting all routes from plugins + app, scan for duplicate method+path combinations:

```
[foundry] Warning: route conflict — POST /api/users registered by both plugin "auth" and plugin "admin"
```

Implementation: Extract registered paths from HttpRegistrar entries. Group by method+path. Warn on duplicates.

### 2.2 Command ID Collision Detection

After collecting all commands, scan for duplicate CommandId values:

```
[foundry] Warning: command conflict — "user:sync" registered by both plugin "auth" and app commands
```

Implementation: Track (CommandId, source) pairs. Detect duplicates after collection.

### 2.3 Service Type Collision Warning

When two providers register the same TypeId in the container, log a warning:

```
[foundry] Warning: service conflict — MyService registered by both plugin "analytics" and plugin "reporting"
```

Implementation: The existing Container uses `no-overwrite` semantics (returns error). This is already safe — the second registration fails. But the error message should mention which plugin caused it.

### 2.4 Config Namespace Convention

Document and enforce a convention for plugin config keys:

```toml
# Plugin config should live under [plugins.{plugin_id}]
[plugins.analytics]
api_key = "..."
retention_days = 90
```

The `config_defaults()` method should validate that plugin defaults only set keys under `plugins.{plugin_id}` (warning if not, not error — to avoid breaking existing plugins).

---

# Phase 3: Plugin Metadata API

## Problem

Cannot inspect what a plugin registered at runtime. Useful for admin dashboards, debugging, and documentation generation.

## Solution

Expose a read-only query API on PluginRegistry:

```rust
impl PluginRegistry {
    fn plugin_routes(&self, id: &PluginId) -> Vec<String>;
    fn plugin_commands(&self, id: &PluginId) -> Vec<CommandId>;
    fn plugin_jobs(&self, id: &PluginId) -> Vec<JobId>;
    fn plugin_channels(&self, id: &PluginId) -> Vec<ChannelId>;
}
```

This requires tracking the source plugin during registration — each registrar action gets tagged with the plugin ID.

### Implementation

Add `plugin_id: Option<PluginId>` to PreparedPlugins registrations. During `prepare_plugins()`, tag each collected item with the source plugin. Store this mapping in PluginRegistry.

---

# Phase 4: Shutdown Lifecycle

## Problem

Plugins have no cleanup hook. The lifecycle is startup-only:

```
manifest() → register() → boot() → [runs] → [process exits]
```

## Solution

Add an optional `shutdown()` method to the Plugin trait:

```rust
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    fn manifest(&self) -> PluginManifest;
    fn register(&self, registrar: &mut PluginRegistrar) -> Result<()>;
    async fn boot(&self, app: &AppContext) -> Result<()> { Ok(()) }
    async fn shutdown(&self, app: &AppContext) -> Result<()> { Ok(()) }  // NEW
}
```

Called during graceful shutdown (after SIGTERM/SIGINT, before process exit). Plugins are shut down in **reverse** dependency order (dependents before dependencies).

### When to implement

This is low priority. Most Rust apps handle cleanup via `Drop`. Add when there's a concrete use case — e.g., a plugin that holds a persistent external connection or needs to flush buffers.

---

# Phase 5: Test Coverage Expansion

### Missing scenarios to add:

| Test | Module |
|------|--------|
| Diamond dependency graph (A→B, A→C, B→D, C→D) | unit |
| Plugin registers guard/job directly (no provider wrapper) | acceptance |
| Two plugins with conflicting route paths → warning | acceptance |
| Two plugins with conflicting command IDs → warning | acceptance |
| Plugin config defaults under `plugins.*` convention | unit |
| Plugin middleware applied before app middleware | acceptance |
| Plugin with all 12 direct registration methods | acceptance |
| Conflicting version requirements in nested deps | unit |

---

# Implementation Status

| Phase | Scope | Status |
|-------|-------|--------|
| Phase 1 | Registration parity (12 new methods) | ✅ Done |
| Phase 2 | Conflict detection (routes, commands, services) | ✅ Done (routes warn; commands/schedules/channels/rules/container already error on duplicates) |
| Phase 3 | Plugin metadata query API | ✅ Done (PluginContributions struct, contributions() on PluginRegistry, enhanced plugin:list output) |
| Phase 4 | Shutdown lifecycle | ✅ Done (shutdown() on Plugin trait, reverse dep order, wired into all run_* methods) |
| Phase 5 | Test coverage expansion | ✅ Done (diamond deps, version conflicts, direct registrations, middleware collection — 11 unit + 1 acceptance) |

---

# Priority & Sequencing

**Phase 1 → Phase 2 → Phase 5 → Phase 3 → Phase 4**

Phase 1 is the most impactful for plugin author DX. Phase 2 prevents the most dangerous failure mode (silent conflicts). Phase 5 should follow immediately to validate Phase 1+2. Phase 3 is nice-to-have for tooling. Phase 4 waits for a real use case.

---

# Files

| File | Role |
|------|------|
| `src/plugin/mod.rs` | Plugin trait, PluginRegistrar, PreparedPlugins, dependency resolution, CLI commands |
| `src/foundation/app.rs` | Bootstrap integration — where plugin registrations are applied |
| `src/foundation/provider.rs` | ServiceRegistrar — the target API that PluginRegistrar now mirrors |
| `tests/plugin_acceptance.rs` | Acceptance tests for plugin system |
| `tests/fixtures/plugin_base/` | Base plugin fixture |
| `tests/fixtures/plugin_consumer_app/` | Consumer app with plugin dependency |
