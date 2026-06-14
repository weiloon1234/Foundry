# Rust Logging System Blueprint (Framework-Level)

## Overview

This document defines a **framework-level logging system** for a modern Rust backend.

Goal:

> Provide structured, production-ready logging as a foundation — not just printing logs.

---

# 🎯 Objective

Build a logging system that:

- Uses **structured JSON logs (NDJSON)**
- Works consistently in **dev and production**
- Supports **request / job / websocket context**
- Captures **errors, panics, and system events**
- Is **viewer-friendly but viewer-agnostic**
- Supports **file and stdout output**

---

# 🧠 Core Philosophy

1. Logs are **structured data**, not strings
2. One format everywhere (no pretty vs JSON split)
3. Framework produces logs, app decides how to view
4. Context is more important than message text
5. Logs must be **searchable, filterable, and parseable**

---

# 📄 Log Format (NDJSON)

Each log entry is **one JSON object per line**.

Example:

```json
{"ts":"2026-04-11T21:40:00Z","level":"INFO","target":"http.request","msg":"Request completed","request_id":"req_123","method":"GET","path":"/api/users","status":200,"duration_ms":18}
```

---

# 🔤 Standard Fields

## Required

- ts → ISO8601 timestamp
- level → TRACE | DEBUG | INFO | WARN | ERROR
- target → logical category (http.request, worker.job, etc)
- msg → human-readable message

## Common Optional Fields

- request_id
- trace_id
- method
- path
- status
- duration_ms
- actor_id
- actor_type
- ip
- job
- job_id
- queue
- connection_id
- channel
- event
- error

---

# 🧱 Output Strategy

## Supported Sinks

### 1. stdout (default)

Best for:
- containers
- cloud logging
- systemd/journald

---

### 2. file sink

Example:

```text
logs/http.log
logs/worker.log
```

Characteristics:
- append-only
- newline-delimited JSON

---

## Not included in v1

- database logging
- remote logging integrations
- log viewer UI

---

# ⚙️ Framework-Level Features

## Must Provide

- log levels
- JSON formatter
- stdout sink
- file sink
- request ID generation
- request logging middleware
- panic logging
- structured field logging
- context propagation

---

# 🧠 Context Model

## Global logger (shared)

```rust
AppState {
    logger: Arc<Logger>,
}
```

---

## Per-request context

```rust
RequestContext {
    request_id: String,
    locale: String,
    actor_id: Option<i64>,
}
```

All logs within a request inherit this context.

---

## Worker context

```rust
JobContext {
    job: String,
    job_id: String,
    attempt: u32,
}
```

---

## WebSocket context

```rust
SocketContext {
    connection_id: String,
    channel: String,
}
```

---

# 🔄 Logging Flow

```text
Event occurs
   ↓
Collect context (request/job/socket)
   ↓
Build structured log event
   ↓
Serialize to JSON
   ↓
Write to sink (stdout/file)
```

---

# 🌐 HTTP Request Logging

Framework should provide middleware that logs:

- method
- path
- status
- duration
- request_id

Example:

```json
{"ts":"...","level":"INFO","target":"http.request","msg":"Request completed","method":"POST","path":"/api/orders","status":201,"duration_ms":32}
```

---

# ⚠️ Error & Panic Handling

Framework MUST capture:

- unhandled HTTP errors
- worker job failures
- scheduler errors
- websocket handler errors
- panics (where possible)

Example:

```json
{"ts":"...","level":"ERROR","target":"http.error","msg":"Unhandled error","error":"connection timeout","request_id":"req_123"}
```

---

# 🧩 Request ID

Each request MUST have a unique ID.

Example:

```text
req_abc123
```

Used for:
- tracing logs
- debugging issues
- correlating events

---

# ⚡ Performance Design

- logging is append-only
- no blocking heavy operations
- avoid large payload logging
- avoid full request body logging

---

# 🔐 Redaction (Future)

Framework should later support masking:

- password
- token
- authorization headers

---

# 🧪 Example Usage

```rust
ctx.logger.info("User logged in", fields! {
    "user_id" => user.id,
    "ip" => ip
});
```

---

# 🚫 What We DO NOT Do

- no pretty log mode
- no mixed formats
- no built-in viewer
- no string-only logs

---

# 🚀 Future Enhancements

- log rotation
- OpenTelemetry integration
- remote sinks (Loki, ELK)
- web-based viewer (app-level)

---

# ✅ Final Summary

This design:

- uses JSON logs everywhere
- supports structured context
- is compatible with modern logging systems
- avoids complexity in v1

---

# 🧠 Final Statement

> Logs are not text. Logs are structured data.
>
> Build them once correctly, and everything else becomes easier.

