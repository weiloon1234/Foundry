# Rust I18n System Blueprint (Shared Frontend + Backend)

## Overview

This document defines the design of a **framework-level i18n system** for a Rust backend that shares the **exact same translation files with frontend (React / i18next)**.

---

# 🎯 Objective

Build an i18n system that:

- Uses **single source of truth** for translations (shared JSON files)
- Works for **both frontend (React/i18next) and backend (Rust)**
- Requires **NO sync / export / duplication**
- Supports **multi-file per locale**
- Supports **parameterized messages**
- Supports **nested JSON (optional)**
- Provides **simple developer experience**:

```rust
t("Something went wrong")
t("Hello, {{name}}", values! { "name" => "WeiLoon" })
```

---

# 🧠 Core Philosophy

1. **Frontend-first compatibility (i18next style)**
2. Backend adapts to frontend format (not the other way)
3. No artificial translation keys (no `common.xxx` required)
4. English string = lookup key
5. File split = organization only, NOT runtime namespace

---

# 📁 Folder Structure (Shared)

```text
/locales
  /en
    common.json
    validation.json
    auth.json
  /ms
    common.json
    validation.json
    auth.json
```

✔ Shared between frontend and backend ✔ Same files, no transformation

---

# 📄 JSON Format (i18next-compatible)

## Example (en/common.json)

```json
{
  "Something went wrong": "Something went wrong",
  "Hello, {{name}}": "Hello, {{name}}"
}
```

## Example (ms/common.json)

```json
{
  "Something went wrong": "Sesuatu telah berlaku ralat",
  "Hello, {{name}}": "Helo, {{name}}"
}
```

---

# 🔤 Interpolation Format

Use:

```
{{variable}}
```

Reason:

- Matches i18next default
- Frontend compatibility out-of-the-box
- No custom parsing rules required

---

# 🧩 Multi-file Strategy

## Problem

Multiple files per locale:

- common.json
- validation.json
- auth.json

But app wants:

```rust
t("Something went wrong")
```

without specifying file or namespace.

---

## Solution

### Backend behavior:

At startup:

1. Scan all files under `/locales/{locale}`
2. Load all JSON files
3. Flatten nested structures (optional)
4. Merge into ONE runtime catalog

---

## Result

```text
en => {
  "Something went wrong": "Something went wrong",
  "Validation failed": "Validation failed",
  ...
}
```

---

# ⚠️ Duplicate Key Policy

Since keys are English strings:

- Duplicate keys across files MUST be handled

Recommended:

- Fail fast OR
- Warn loudly during startup

---

# 🧠 Runtime Model (IMPORTANT)

## Global Catalogs, Per-Request Locale

The framework MUST NOT use a mutable process-wide current locale.

That model is wrong for concurrent web servers.

### Wrong model

```text
GLOBAL_LOCALE = "ms"
```

Because another request in the same process may need:

```text
GLOBAL_LOCALE = "en"
```

at the same time.

---

## Correct model

### Global app state

Load all translation catalogs once at startup and keep them in shared read-only memory.

Example:

```rust
AppState {
    i18n: Arc<I18nManager>,
}
```

---

### Per-request locale context

Each request resolves its own locale from:

- Accept-Language header
- custom locale header
- cookie
- authenticated user preference
- default fallback locale

Then the resolved locale is stored in request context.

Example:

```rust
RequestContext {
    locale: "ms",
}
```

---

## Translation flow

```text
Request arrives
   ↓
Resolve locale
   ↓
Store locale in request context
   ↓
Lookup translation from global in-memory catalog
   ↓
Interpolate variables
   ↓
Return translated string
```

---

## Runtime API example

```rust
ctx.t("Something went wrong")
ctx.t("Hello, {{name}}", values! { "name" => "WeiLoon" })
```

---

## Performance design

### Startup

At startup:

- scan translation files
- parse JSON
- merge catalogs
- optionally flatten nested JSON
- optionally precompile interpolation templates

### Request time

At request time:

- detect locale
- perform in-memory lookup
- interpolate values

This is fast and usually the correct design.

---

## Why in-memory is recommended

- avoids disk IO on every request
- avoids reparsing JSON on every request
- enables O(1)-like catalog lookup
- keeps request path lightweight

Translation catalogs are usually tiny compared to:

- DB pools
- request buffers
- websocket state
- application caches

So this is normally not a performance concern.

---

## Optional optimization

For parameterized translations like:

```text
Hello, {{name}}
```

The framework may precompile templates once at startup, instead of reparsing placeholder patterns every request.

This is an optimization, not a v1 requirement.

---

# ⚙️ Framework-Level Features

## Must Provide

- register\_locales([...])
- default\_locale(...)
- fallback\_locale(...)
- translation\_path(...)

---

## Example

```rust
App::builder()
    .register_locales(["en", "ms", "zh-CN"])
    .default_locale("en")
    .fallback_locale("en")
    .translation_path("locales")
```

---

# 🔄 Runtime Behavior

## Basic

```rust
t("Something went wrong")
```

## With parameters

```rust
t("Hello, {{name}}", values! { "name" => "WeiLoon" })
```

---

# 🌍 Locale Resolution

Framework should support:

- request-based locale (header/cookie)
- manual override
- fallback locale

---

## Example

```rust
ctx.t("Something went wrong")
```

---

# 🧱 Internal Design

## Core structure

```rust
I18nManager {
    default_locale: String,
    fallback_locale: String,
    catalogs: HashMap<Locale, Catalog>
}
```

## Catalog

```rust
HashMap<String, String>
```

---

# ⚡ Performance Design

- Load once at startup
- Merge and flatten once
- Store in memory
- Runtime lookup = O(1)

---

# 🧩 Nested JSON Support (Optional)

Example:

```json
{
  "errors": {
    "Something went wrong": "Something went wrong"
  }
}
```

Backend should flatten into:

```text
"Something went wrong" => "Something went wrong"
```

---

# 🧪 Validation Example

```rust
validator.required("email")
    .message("The {{field}} field is required")
```

---

# 🎯 Frontend Integration (IMPORTANT)

## React (i18next)

Frontend uses SAME files:

```js
useTranslation(['common', 'validation'])

 t('Something went wrong')
```

---

## Key Note

- Frontend may still use namespaces
- Backend ignores namespaces (merged)

---

# 🚫 What We DO NOT Do

- No export scripts
- No sync pipeline
- No duplicate translation files
- No forced key naming like `common.xxx`

---

# ⚠️ Trade-offs

## Pros

- Single source of truth
- Clean developer experience
- No duplication
- Frontend/backend consistency

## Cons

- English string becomes key
- Changing English breaks lookup
- Duplicate text collision risk

---

# 🚀 Future Enhancements

- i18n\:missing command
- i18n\:extract
- duplicate key detection
- runtime hot reload (dev only)

---

# ✅ Final Summary

This design:

- Uses i18next-compatible JSON
- Shares files between frontend/backend
- Merges multiple files into one runtime catalog
- Uses `t("...")` with natural language keys
- Keeps framework-level configuration simple

---

# 🧠 Final Statement

> One translation source. One format. Zero duplication.
>
> Frontend and backend speak the same language.

