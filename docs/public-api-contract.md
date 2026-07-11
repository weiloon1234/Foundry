# Foundry Public API Contract

This document defines the supported import layers and compatibility expectations for Foundry consumer applications. The generated [API surface](api/index.md) lists the current public items; this contract explains which surfaces consumers should depend on and how changes are communicated.

## Supported import layers

Prefer imports in this order:

1. `foundry::prelude::*` for the common application-facing types used across handlers, providers, models, jobs, and tests.
2. `foundry::<Item>` for stable convenience re-exports from the crate root.
3. `foundry::<module>::<Item>` when an explicit module makes ownership clearer or an item is intentionally not in the prelude.

Anything reachable only through `foundry::__private` or `foundry::__reexports` is framework implementation support. Consumer code must not depend on those modules unless generated Foundry code references them.

## Compatibility guarantees

Foundry treats the following as public contracts:

- Public structs, enums, traits, functions, methods, macros, and typed identifiers.
- Configuration keys, accepted values, defaults, validation rules, and environment overlays.
- Database tables and columns created by Foundry-owned migrations.
- Serialized request, response, event, job, notification, and WebSocket payloads.
- The normalized contract manifest and generated TypeScript SDK output.
- CLI command names, arguments, exit behavior, and documented file-generation output.

Within the current `0.x` release line, incompatible changes may still be necessary while the framework matures. They must be called out in `CHANGELOG.md` and accompanied by a dated consumer-impact document when consumers need to update code, configuration, schema, generated clients, or deployment behavior.

## What counts as breaking

Examples include:

- Removing, renaming, or moving a public item without a compatibility re-export.
- Adding a required trait method without a default implementation.
- Changing a public function signature, return contract, or error behavior in a way that requires consumer edits.
- Renaming a configuration key, changing its type, or materially changing a default.
- Changing persisted or wire-format field names, identifier semantics, or contract-manifest structure.
- Changing generated TypeScript names or removing generated files used by consumers.

Additive APIs, optional configuration, new enum variants on explicitly non-exhaustive types, new generated files, and new first-class modules are normally non-breaking. They are still documented when consumers should opt in, regenerate artifacts, or change deployment wiring.

## Deprecation and migration

When practical, Foundry uses this sequence:

1. Add the replacement API and document it.
2. Keep the previous API as a deprecated compatibility layer.
3. Provide a consumer migration example.
4. Remove the deprecated surface in a later explicitly documented release.

Correctness or security fixes may require a direct breaking change. In that case, the consumer-impact document must explain the old behavior, the new behavior, the required action, and how to verify the migration.

## Generated artifacts

Generated API documentation and TypeScript output are owned by their generators. Consumers should not hand-edit generated files.

- Regenerate API documentation with `make api-docs`.
- Regenerate TypeScript contracts with Foundry's `types:export` command.
- Treat `CONTRACT_MANIFEST_VERSION` as the compatibility boundary for normalized contract consumers.

Application-specific wrappers may be built on generated output, but they should live outside generator-owned paths.

## Consumer upgrade checklist

Before upgrading Foundry:

1. Read the relevant `CHANGELOG.md` section and dated file under `docs/consumer-impact/`.
2. Apply documented code, configuration, database, and deployment changes.
3. Regenerate API/TypeScript artifacts when the release changes contracts or generators.
4. Run the consumer application's tests and Foundry fixture checks on the target Rust toolchain.

