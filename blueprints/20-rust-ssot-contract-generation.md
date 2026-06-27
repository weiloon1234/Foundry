# Rust SSOT Contract Generation Review

> **Status:** Implemented foundation
> **Created:** 2026-06-27
> **Purpose:** Evaluate and implement the first Foundry contract-manifest foundation for Rust as the single source of truth for backend contracts, frontend business actions, generated SDKs, and future adapters.

---

## Executive Verdict

Foundry is directionally close, and the first contract-manifest foundation now exists. The remaining long-term work is to keep moving generators and adapters behind that manifest instead of adding more direct macro-to-TypeScript paths.

Rust already owns many important facts: DTO shapes, route IDs, HTTP methods, path params, permissions, AppEnum metadata, validation attributes, multipart extraction, and some OpenAPI schema data. However, these facts are gathered by several separate systems:

- `HttpRegistrar::collect_route_manifest()` creates a route-oriented manifest.
- `RouteDoc` drives OpenAPI request/response metadata.
- `#[derive(Validate)]` generates runtime validation plus separate TypeScript validation metadata.
- `#[derive(ApiSchema)]` generates OpenAPI JSON Schema plus TypeScript registration.
- `types:export` writes a normalized `FoundryContractManifest.json` and generates a pure SDK layer from it.
- Compatibility route helpers still render from route metadata while the form helper layer is kept available.
- WebSocket channels expose descriptors and can declare typed incoming/outgoing event payload contracts.

That means the current Rust-to-TypeScript work is useful, but it should not become the core architecture. The durable direction should be:

```text
Rust definitions
    -> normalized Foundry contract manifest
        -> pure TypeScript SDK
        -> optional React/Vue/Flutter adapters
        -> OpenAPI
        -> validation metadata
        -> realtime contracts
```

The problem is not "Rust to TypeScript" by itself. The core fix is the stable intermediate contract model: new generators should target that manifest first.

---

## Implemented Foundation

- Added `src/contract/` with `ContractManifest`, contract actions, HTTP/WebSocket transports, schemas, validation schemas, value kinds, and realtime channel/event slots.
- Enriched `#[derive(Validate)]` metadata so every DTO field carries a contract value kind.
- `types:export` now writes `FoundryContractManifest.json`.
- `types:export` now emits `FoundryErrors.ts`, `FoundrySdk.ts`, `FoundryClient.ts`, and per-action `sdk/*.ts` files.
- `client_export(false)` now disables both legacy route helper generation and SDK action generation.
- OpenAPI now has `generate_openapi_spec_from_contract(...)`, and the existing generator delegates through a manifest conversion.
- Existing `FoundryEndpoint` route helpers remain available as the form-state adapter layer.
- WebSocket descriptors can be projected into contract realtime channel entries, including declared typed payload and no-payload events.

---

## Review Matrix

| Item | Already implemented | Partially implemented | Missing | Architecture should change | Recommendation |
|------|---------------------|-----------------------|---------|----------------------------|----------------|
| 1. Single Source of Truth | Rust owns DTOs, route metadata, permissions, AppEnum metadata, validation derives, upload extraction, generated error shapes, realtime declarations, and the generated contract manifest. | Existing compatibility route helpers still read route metadata directly. | Explicit action names separate from route IDs. | Continue migrating | Use `FoundryContractManifest.json` as the generator boundary for new outputs. |
| 2. Contract Generation | `ContractManifest` now covers schemas, actions, validation, transport, permissions, standard errors, file semantics, and realtime contracts. | OpenAPI and SDK use the manifest path; compatibility route helpers remain. | Non-HTTP action transports and richer schema reuse. | Mostly done foundation | Keep new generators behind the manifest. |
| 3. Generated Frontend SDK | Pure SDK runtime/client and per-action SDK modules are generated. | `FoundryEndpoint` remains available as form-state compatibility output. | Framework-specific React/Vue/Flutter adapters. | Done for core SDK | Prefer `createFoundryClient(...)` for new frontend code. |
| 4. Runtime Domain Types | Contract value kinds now include scalar, array, object, file/file-list, date/time, decimal, UUID, JSON, page, and error. | Some kinds still need deeper framework-level DTO conventions. | Dedicated resource/page/error DTO conventions. | Partially | Extend contract value kinds as domain types mature. |
| 5. DTO Philosophy | Foundry permits action-specific requests and shared response DTOs. | Generated route aliases can obscure the shared response type. | A documented and generator-supported convention. | Mildly | Keep requests action-specific; make shared response/resource DTOs the default convention. |
| 6. Frontend Independence | Generated TS is headless and not tied to React/Vue. | `FoundryEndpoint` mixes SDK and form state. | Clear separation between core SDK and framework adapters. | Yes | Core emits pure TS; adapters emit forms/hooks/composables later. |
| 7. Code Generation Quality | Modular generated files, typed route params, AppEnum metadata, validation metadata, manifest cleanup, SDK runtime, and typed error runtime. | Scales moderately, but generation is still string-template-heavy. | Manifest compiler utilities and adapter generation. | Yes | Generate all frontend/OpenAPI outputs from the manifest. |
| 8. Future Direction | Many primitives are in place early enough to redirect. | Current route helper design can still be replaced. | Stable action model, transport model, error model, realtime payload model. | Yes, soon | Redesign now before consumers depend on route-helper APIs. |

---

## 1. Single Source Of Truth

**Status:** Implemented foundation; compatibility outputs still need migration

Rust is already the source for many backend facts:

- DTOs derive `ApiSchema`, `ts_rs::TS`, and sometimes `Validate`.
- routes carry typed route IDs, methods, path params, guards, permissions, request schemas, and response schemas.
- AppEnum owns stable enum keys and frontend-ready metadata.
- `UploadedFile` and `FromMultipart` establish backend upload semantics.
- `types:export` writes a normalized `FoundryContractManifest.json`.
- `types:export` writes a generated SDK client/runtime and typed error runtime.
- WebSocket channel options can declare typed incoming/outgoing realtime contracts.

Rust is still not the only source of truth for every future contract concern:

- TypeScript output depends on `ts-rs` attributes such as `#[ts(type = "...")]`, `#[ts(optional)]`, and `#[ts(export_to = "...")]`.
- compatibility route helpers still read route metadata directly.
- route IDs still double as generated default action names.
- WebSocket handlers still receive `serde_json::Value`; typed event declarations are contract metadata, not handler-level decoding yet.

**Recommendation:** Keep `FoundryContractManifest.json` as the SSOT boundary for new generators. Continue migrating compatibility outputs behind the manifest, and add explicit action naming plus typed WebSocket handler ergonomics next.

---

## 2. Contract Generation

**Status:** Implemented foundation; non-HTTP depth should expand

Foundry now has a normalized manifest with these pieces:

- `RouteManifestEntry` has route ID, path, method, params, guard, permissions, summary, request, and responses.
- `TsValidationSchema` has fields, rules, messages, and attributes.
- `ApiSchema` produces schema JSON.
- WebSocket channel descriptors expose channel ID, auth, permissions, presence, replay, and client-event settings.

`ContractManifest` now represents:

- DTO schemas
- endpoints/actions
- validation
- transport
- permissions
- file/multipart semantics
- standard error contracts
- realtime channel contracts

Still missing:

- explicit non-HTTP action transports
- handler-level typed WebSocket decoding

**Recommendation:** Keep the core unit as a business action, with HTTP as one possible transport:

```text
ContractAction {
  id,
  action_name,
  request_dto,
  response_dto,
  auth,
  permissions,
  validation,
  transport: Http | WebSocket | Command | Future(...)
}
```

OpenAPI is now available through `generate_openapi_spec_from_contract(...)`; it should remain an adapter output, not the canonical model.

---

## 3. Generated Frontend SDK

**Status:** Implemented for core TypeScript SDK; adapters missing

The current `types:export` output now produces a pure SDK layer:

- `FoundrySdk.ts` owns transport execution.
- `FoundryClient.ts` binds a transport once.
- `sdk/*.ts` files expose per-action factories.
- generated client calls are business-action shaped, for example `api.userPortalLogin(dto)`.

The compatibility route helper output still exists for form-state use. It should be treated as an adapter layer, not the core SDK.

**Recommendation:** The core generated SDK should continue to expose pure TypeScript business actions:

```ts
await SubmitProfileForm(requestDto)
```

or, with explicit client binding:

```ts
const api = createFoundryClient({ transport });
await api.profile.submit(requestDto);
```

Then adapters can wrap that core:

- React hooks
- Vue composables
- form-state helpers
- Flutter client generation

The current route helper layer can inform the implementation, but it should not be treated as the final SDK contract.

---

## 4. Runtime Domain Types

**Status:** Implemented as a contract foundation; deeper conventions remain

Foundry already has useful runtime types:

- `UploadedFile`
- `ModelId<M>`
- typed semantic IDs such as `RouteId`, `PermissionId`, `GuardId`
- temporal helpers under `support::datetime`
- datatable request/response DTOs
- storage metadata types

Foundry now has `ContractValueKind`, which gives generators the first contract-level signal for how fields behave across transports. Naming is secondary, but canonical contract kinds should continue to cover:

- file/upload
- date
- datetime
- local datetime
- decimal
- uuid/model id
- json
- paginated page
- validation error
- application error

The important part is that these types carry generator semantics. For example:

```rust
avatar: FoundryFile
documents: Vec<FoundryFile>
```

should make the contract manifest mark the action as requiring multipart transport. The frontend SDK should then construct `FormData` automatically, without the app developer caring about `FormData`.

**Recommendation:** Do not bolt this onto TypeScript rendering. Add it to the contract manifest first, then let each generator decide how to represent the type.

---

## 5. DTO Philosophy

**Status:** Partially implemented

The current architecture permits the right style:

- request DTOs can be endpoint/action-specific.
- response DTOs can be shared resource/view DTOs.
- route response aliases in generated TypeScript can point back to shared response types.

But the architecture does not strongly encourage this philosophy yet. Because route helpers generate per-route aliases such as `UserPortalLoginResponse`, developers may be nudged toward endpoint-shaped response naming even when a shared resource DTO would be better.

The proposed DTO philosophy is good:

- requests are usually action-specific: `CreateUserRequest`, `UpdateProfileRequest`.
- responses are usually shared views/resources: `UserDto`, `UserProfileDto`, `OrderDto`.
- route/action-specific response DTOs should exist only when the response is truly unique.

**Recommendation:** Document this as a Foundry convention and reflect it in contract generation. The manifest should distinguish:

- action request type
- action response type
- reusable resource/view DTO

Generated SDK aliases can remain ergonomic, but the canonical response type should stay visible and reusable.

---

## 6. Frontend Independence

**Status:** Mostly implemented, with one caution

Foundry is not currently coupled to React, Vue, or another frontend framework. The generated TypeScript runtime is headless and accepts any client with a `request(config)` method.

The caution is that `FoundryEndpoint` mixes SDK concerns with UI/form concerns:

- `busy`
- `errors`
- `response`
- `status`
- `subscribe()`
- `validateForm()`
- `submitForm()`

This is framework-independent, but it is still UI-state-shaped.

**Recommendation:** Split generated frontend output into layers:

1. pure TypeScript SDK core: stateless actions, transport, DTOs, errors.
2. optional form helper layer.
3. optional React/Vue/Flutter adapters.

Core Foundry should own layer 1. Everything else should be adapter output.

---

## 7. Code Generation Quality

**Status:** Partially implemented

Strengths:

- generated files are modular.
- route helpers are per-route, which can scale better than one giant endpoint file.
- TypeScript route params are typed.
- AppEnum metadata is frontend-friendly.
- validation metadata improves IDE and form integration.
- generated output is headless.
- manifest-owned cleanup avoids deleting manual files.

Weaknesses:

- some TypeScript is still generated directly from route structs, inventory registrations, and string templates.
- `ts-rs` attributes leak into Foundry's public contract story.
- missing exported Rust types degrade to `unknown` in route helpers.
- action naming is derived from route IDs instead of explicit business actions.
- compatibility route helpers are form-oriented.
- OpenAPI uses the manifest path, but route helpers still have a direct metadata path.
- WebSocket contracts are metadata declarations; handler decoding is still value-based.

**Recommendation:** Move generation behind a manifest compiler:

```text
Rust compile-time metadata
    -> FoundryContractManifest
        -> TypeScript SDK generator
        -> TypeScript type generator
        -> OpenAPI generator
        -> adapter generators
```

Generated TypeScript should be ESM, modular, side-effect-light, and organized by action/domain. The SDK should expose business names first and HTTP details only as debug/escape-hatch metadata.

---

## 8. Future Direction

**Status:** Foundation is in place; several API decisions should still settle before consumer APIs are frozen

The most important redesign was the contract manifest. Now that the foundation exists, new route helpers, React hooks, validation exports, or WebSocket helpers should build from this layer so the project does not grow multiple competing public surfaces.

Decisions that will become expensive to change later:

- treating route IDs as action names.
- exposing `FoundryEndpoint` as the main frontend abstraction.
- keeping `ts-rs` attributes as the visible TypeScript contract API.
- generating OpenAPI and TypeScript from separate schema/rule paths.
- relying on JS runtime file detection instead of Rust-declared transport semantics.
- leaving WebSocket handler payloads as untyped `serde_json::Value`.
- making `client_export` route-specific before defining what "client action export" means.
- embedding validation semantics in TypeScript runtime code before stabilizing a shared validation rule manifest.

Completed sequence:

1. Defined `FoundryContractManifest` as the normalized intermediate model.
2. Made HTTP route collection produce contract actions.
3. Moved validation metadata into a shared validation contract model.
4. Defined contract scalar/domain kinds, including file/upload and error/page types.
5. Generated OpenAPI from the manifest.
6. Generated a pure TypeScript SDK from the manifest.
7. Added typed WebSocket event/payload declarations.

Recommended next sequence:

1. Add explicit action naming separate from route IDs.
2. Move current form endpoint helpers behind an optional adapter layer.
3. Add typed WebSocket handler decoding ergonomics.
4. Add React/Vue/Flutter adapters on top of the pure SDK.

---

## Challenge To The Assumptions

Rust should be the SSOT for application contracts, not for every frontend decision.

Foundry should generate:

- DTOs
- action functions
- transport handling
- validation metadata
- permission metadata
- realtime contracts
- typed errors

Foundry should not try to generate all frontend business orchestration or UI behavior. Frontend code still owns interaction state, view composition, optimistic UI, accessibility behavior, and product-specific flows.

The strongest version of the vision is:

```text
Rust owns contract truth.
Generated SDK owns safe communication.
Frontend owns user experience.
```

---

## Consumer Impact

Consumer impact for the implemented foundation is logged in `docs/consumer-impact/2026-06-27-rust-ssot-contract-generation.md`.

Remaining future impact should be logged separately if these follow-up changes land:

- explicit action names separate from route IDs.
- optional adapter packages for form helpers, React, Vue, or Flutter.
- typed WebSocket handler decoding instead of raw `serde_json::Value`.
- narrower Foundry-owned DTO export attributes if `ts-rs` becomes a fully internal implementation detail.
