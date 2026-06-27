# Rust SSOT Contract Generation Impact

Date: 2026-06-27

This change introduces a normalized Foundry contract manifest and a generated pure TypeScript SDK layer. Existing route helper output remains available for compatibility, but new frontend code should prefer the SDK client.

## TypeScript Generation

- `types:export` now writes `FoundryContractManifest.json`.
- `types:export` now writes `FoundryErrors.ts`, `FoundrySdk.ts`, `FoundryClient.ts`, and per-action files under `sdk/`.
- `index.ts` re-exports the SDK client/runtime, typed error helpers, and per-action SDK modules.
- The generated cleanup manifest now tracks the contract JSON file as well as generated TypeScript files.
- `#[derive(ApiSchema)]` and `#[derive(AppEnum)]` now register JSON schemas for inclusion in the contract manifest.

## Routing And Actions

- HTTP routes collected for TypeScript export now also become contract actions.
- Action names are generated from route IDs and stored in the contract manifest.
- Request/response DTO names in actions now resolve to JSON schema entries in the same manifest.
- `client_export(false)` / `without_client_export()` now disables both form route helpers and SDK action generation for that route.

## Validation

- `#[derive(Validate)]` now exports value-kind metadata for every DTO field, including unvalidated fields.
- Generated validation metadata can identify scalar, array, object, file, file-list, date/time, decimal, UUID, JSON, page, and error-shaped fields.
- Contract actions infer multipart transport from file/file-list request fields or file validation rules.

## DTOs

- Existing DTO TypeScript files remain generated as before.
- SDK action files use route-specific SDK aliases while preserving imports of the canonical DTO types.
- Shared response DTOs remain supported and are the preferred long-term convention.

## Files And Transport

- The contract manifest records HTTP body kind as `none`, `json`, `multipart`, or `unknown`.
- The generated SDK automatically sends multipart `FormData` for manifest-declared multipart actions.
- The SDK retains a file-like fallback for JSON actions to avoid surprising runtime behavior during transition.

## Realtime

- The contract model now includes realtime channel entries.
- Existing WebSocket channel descriptors can be converted into contract realtime channels.
- `WebSocketChannelOptions` can now declare typed incoming/outgoing event payloads, plus no-payload events, for contract export.

## Errors

- `types:export` now writes `FoundryErrors.ts` with standard error codes, validation error shapes, and `FoundrySdkError`.
- The SDK runtime centralizes transport response handling and normalizes thrown transport errors into `FoundrySdkError`.
