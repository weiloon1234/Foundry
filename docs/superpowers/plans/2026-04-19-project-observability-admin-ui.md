# Project Observability Admin UI Plan

> **For agentic workers:** Build this at the application/project level, not inside Foundry core. Reuse Foundry's shipped read-only `/_foundry/*` endpoints as the data source. Keep the UI view-only in this phase.

**Goal:** Add a view-only admin area with two dashboards:

1. `Admin > Observability > Jobs`
2. `Admin > Observability > WebSocket`

This should feel closer to a focused operations console than a full Laravel Telescope clone. The available Foundry endpoints are strong for summaries, recent failures, channel activity, presence, and replay history. They are not deep enough yet for a full trace explorer, queue inspector, or admin actions UI.

**Recommendation:** Ship exactly two dashboards in v1:

- `Jobs Dashboard`
- `WebSocket Dashboard`

Do **not** build a broad "Telescope-style everything dashboard" yet. Keep scope tight and high-signal.

---

## Why This Shape

Foundry already exposes the right read-only data for:

- job status counts
- recent failed/retried jobs
- websocket channel registry
- live websocket presence
- recent websocket replay history
- global and per-channel websocket counters
- runtime summary and liveness/readiness

The current data does **not** support:

- per-job lifecycle timeline
- pending/running queue explorer
- retry / replay / delete actions
- per-room websocket analytics
- per-connection websocket inspection
- cross-node aggregation in-app

So the best product is a small ops console with two deep pages, not a shallow mega-dashboard.

---

## Information Architecture

### Admin Menu

- `Admin`
- `Observability`
- `Jobs`
- `WebSocket`

Optional later:

- `Overview`

Do not ship `Overview` in v1 unless the project specifically wants a landing page. The two dashboards are enough.

### Routes

Use project-level admin routes such as:

- `/admin/observability/jobs`
- `/admin/observability/websocket`

Keep route naming aligned with the existing admin route conventions in the project.

---

## Security Model

This UI is **view-only**.

### Requirements

- Protect the project admin pages with the project's normal admin auth.
- Protect the Foundry endpoints behind an observability guard/permission in production.
- The frontend must treat all Foundry endpoint responses as internal admin-only data.

### Explicit non-goals

- no retry button
- no replay button
- no disconnect user action
- no clear history action
- no queue mutation controls

If the project wants actions later, treat that as a separate phase with explicit audit and authorization design.

---

## Dashboard 1: Jobs

## Purpose

Give operators a quick answer to:

- Are jobs healthy right now?
- Are retries or dead letters increasing?
- Which queues are failing?
- What are the latest failed jobs and errors?

## Data Sources

- `GET /_foundry/runtime`
- `GET /_foundry/jobs/stats`
- `GET /_foundry/jobs/failed`
- Optional supporting strip:
  - `GET /_foundry/health`
  - `GET /_foundry/ready`

## Page Layout

### Row 1: Status + KPI cards

Widgets:

- `Runtime Health`
- `Jobs Enqueued`
- `Jobs Started`
- `Jobs Succeeded`
- `Jobs Retried`
- `Jobs Dead Lettered`
- `Expired Lease Requeues`
- `Scheduler Leader`

### Row 2: Charts / summaries

Widgets:

- `Job Status Breakdown`
- `Scheduler Activity`

### Row 3: Operations table

Widgets:

- `Recent Failed / Retried Jobs`

### Optional footer strip

Widgets:

- `Readiness`
- `Backend`
- `Bootstrap Complete`

---

## Jobs Dashboard Widgets

### Widget: Runtime Health

**Source:**

- `/_foundry/health`
- `/_foundry/ready`

**Show:**

- liveness badge: `Healthy` / `Unhealthy`
- readiness badge: `Ready` / `Degraded`
- small text for failed readiness probes when degraded

**UI notes:**

- badge color only is not enough; include text
- make this compact and always visible at top-left

### Widget: Jobs Enqueued

**Source:**

- `/_foundry/runtime`

**Field:**

- `jobs.enqueued_total`

**Show:**

- cumulative total

### Widget: Jobs Started

**Source:**

- `/_foundry/runtime`

**Field:**

- `jobs.started_total`

**Show:**

- cumulative total

### Widget: Jobs Succeeded

**Source:**

- `/_foundry/runtime`

**Field:**

- `jobs.succeeded_total`

**Show:**

- cumulative total
- optional derived success ratio if you also compute from totals in UI

### Widget: Jobs Retried

**Source:**

- `/_foundry/runtime`
- `/_foundry/jobs/stats`

**Fields:**

- `jobs.retried_total`
- stats row where `status = "retried"`

**Show:**

- cumulative total card
- status distribution chip

### Widget: Jobs Dead Lettered

**Source:**

- `/_foundry/runtime`
- `/_foundry/jobs/stats`

**Fields:**

- `jobs.dead_lettered_total`
- stats row where `status = "dead_lettered"`

**Show:**

- cumulative total card
- alert-styled number if non-zero

### Widget: Expired Lease Requeues

**Source:**

- `/_foundry/runtime`

**Field:**

- `jobs.expired_requeues_total`

**Show:**

- cumulative total

### Widget: Scheduler Leader

**Source:**

- `/_foundry/runtime`

**Fields:**

- `scheduler.leader_active`
- `scheduler.executed_schedules_total`
- `scheduler.ticks_total`

**Show:**

- leader badge: `Leader` / `Follower`
- executed schedules total
- tick total as secondary text

### Widget: Job Status Breakdown

**Source:**

- `/_foundry/jobs/stats`

**Response shape:**

```json
{
  "stats": [
    { "status": "succeeded", "count": 123 },
    { "status": "retried", "count": 7 },
    { "status": "dead_lettered", "count": 2 }
  ]
}
```

**Show:**

- donut or horizontal bar chart
- total count above chart
- legend with counts

**Recommendation:**

- use explicit status ordering:
  - `succeeded`
  - `retried`
  - `dead_lettered`
  - anything else after

### Widget: Recent Failed / Retried Jobs

**Source:**

- `/_foundry/jobs/failed`

**Response shape:**

```json
{
  "failed_jobs": [
    {
      "job_id": "...",
      "queue": "default",
      "status": "retried",
      "attempt": 2,
      "error": "...",
      "started_at": "...",
      "completed_at": "...",
      "duration_ms": 1200,
      "created_at": "..."
    }
  ]
}
```

**Show in table columns:**

- `job_id`
- `queue`
- `status`
- `attempt`
- `duration_ms`
- `created_at`

**Row interaction:**

- open right drawer or modal

**Drawer content:**

- full `error`
- `started_at`
- `completed_at`
- `duration_ms`
- raw JSON block for debugging

**Important note:**

- this endpoint is limited to the latest 50 rows
- the UI should state: `Showing latest 50 failed/retried jobs`

### Widget: Runtime Footer Strip

**Source:**

- `/_foundry/runtime`

**Fields:**

- `backend`
- `bootstrap_complete`

**Show:**

- backend type
- bootstrap complete badge

---

## Jobs UX Rules

- Poll every `10s`
- Do not auto-refresh while a details drawer is open unless the project explicitly wants live mode
- Show last refresh timestamp
- Preserve current table sort/filter across refreshes
- Empty state for failed jobs:
  - `No failed or retried jobs found`

### Derived client-side helpers

These are useful but not required from Foundry:

- `success_rate = succeeded_total / started_total`
- `failure_pressure = retried_total + dead_lettered_total`
- `has_alert = dead_lettered_total > 0`

---

## Dashboard 2: WebSocket

## Purpose

Give operators a quick answer to:

- Which websocket channels exist?
- Which channels are active right now?
- Who is present on presence channels?
- What recent events have been replayed?
- Which channels are hot versus idle?

## Data Sources

- `GET /_foundry/ws/stats`
- `GET /_foundry/ws/channels`
- `GET /_foundry/ws/presence/:channel`
- `GET /_foundry/ws/history/:channel`
- Optional support:
  - `GET /_foundry/runtime`
  - `GET /_foundry/health`
  - `GET /_foundry/ready`

## Page Layout

### Row 1: KPI cards

Widgets:

- `Active Connections`
- `Active Subscriptions`
- `Inbound Messages`
- `Outbound Messages`
- `Opened Connections`
- `Closed Connections`

### Row 2: Channel activity

Widgets:

- `Top Active Channels`
- `Channel Registry`

### Row 3: Drilldown

Widgets:

- `Channel Detail Drawer`

Tabs inside drawer:

- `Stats`
- `Presence`
- `Recent History`
- `Config`

---

## WebSocket Dashboard Widgets

### Widget: Active Connections

**Source:**

- `/_foundry/ws/stats`

**Field:**

- `global.active_connections`

**Show:**

- current active connection count

### Widget: Active Subscriptions

**Source:**

- `/_foundry/ws/stats`

**Field:**

- `global.active_subscriptions`

**Show:**

- current active subscription count

### Widget: Inbound Messages

**Source:**

- `/_foundry/ws/stats`

**Field:**

- `global.inbound_messages_total`

**Show:**

- cumulative total

### Widget: Outbound Messages

**Source:**

- `/_foundry/ws/stats`

**Field:**

- `global.outbound_messages_total`

**Show:**

- cumulative total

### Widget: Opened Connections

**Source:**

- `/_foundry/ws/stats`

**Field:**

- `global.opened_total`

**Show:**

- cumulative total

### Widget: Closed Connections

**Source:**

- `/_foundry/ws/stats`

**Field:**

- `global.closed_total`

**Show:**

- cumulative total

### Widget: Top Active Channels

**Source:**

- `/_foundry/ws/stats`

**Fields per channel:**

- `id`
- `active_subscriptions`
- `subscriptions_total`
- `unsubscribes_total`
- `inbound_messages_total`
- `outbound_messages_total`

**Show:**

- ranked list or compact bar chart

**Default sort:**

- `active_subscriptions desc`
- tie-break by `outbound_messages_total desc`

**Useful chips:**

- `Hot`
- `Idle`
- `Presence`
- `Auth`

These chips should be enriched by joining with `/_foundry/ws/channels`.

### Widget: Channel Registry

**Source:**

- `/_foundry/ws/channels`
- `/_foundry/ws/stats`

**Registry response shape:**

```json
{
  "channels": [
    {
      "id": "chat",
      "presence": true,
      "replay_count": 10,
      "allow_client_events": false,
      "requires_auth": true,
      "guard": "api",
      "permissions": ["chat:read"]
    }
  ]
}
```

**Stats response shape:**

```json
{
  "global": { "...": "..." },
  "channels": [
    {
      "id": "chat",
      "subscriptions_total": 200,
      "unsubscribes_total": 180,
      "active_subscriptions": 20,
      "inbound_messages_total": 5000,
      "outbound_messages_total": 20000
    }
  ]
}
```

**Merge by `id` in UI and show columns:**

- `id`
- `presence`
- `replay_count`
- `allow_client_events`
- `requires_auth`
- `guard`
- `permissions`
- `active_subscriptions`
- `inbound_messages_total`
- `outbound_messages_total`

**Table features:**

- search by channel id
- filter by:
  - presence enabled
  - requires auth
  - active only
  - idle only
- sort by:
  - active subscriptions
  - outbound messages
  - channel id

### Widget: Channel Detail Drawer

Open from clicking a channel row.

#### Tab: Stats

**Source:**

- merged channel row from `/_foundry/ws/stats` + `/_foundry/ws/channels`

**Show:**

- active subscriptions
- total subscriptions
- total unsubscribes
- inbound messages
- outbound messages
- replay count
- auth requirement
- guard
- permissions

#### Tab: Presence

**Source:**

- `/_foundry/ws/presence/:channel`

**Only show the tab if `presence = true`**

**Response shape:**

```json
{
  "channel": "chat",
  "count": 3,
  "members": [
    { "actor_id": "user_1", "joined_at": 1713456789 }
  ]
}
```

**Show:**

- member count
- member list
- columns:
  - `actor_id`
  - `joined_at`

**Formatting:**

- convert `joined_at` to readable local timestamp if it is epoch seconds
- show raw value in tooltip if needed

#### Tab: Recent History

**Source:**

- `/_foundry/ws/history/:channel?limit=20`

**Response shape:**

```json
{
  "channel": "chat",
  "messages": [
    {
      "channel": "chat",
      "event": "message",
      "room": null,
      "payload_size_bytes": 245
    }
  ]
}
```

Or when payloads are enabled:

```json
{
  "channel": "chat",
  "messages": [
    {
      "channel": "chat",
      "event": "message",
      "room": null,
      "payload": { "...": "..." }
    }
  ]
}
```

**Show in table/list:**

- `event`
- `room`
- `payload_size_bytes` or payload preview

**Detail behavior:**

- if payload exists, show expandable JSON viewer
- if payload is redacted, show `Payload hidden by server config`

**Important note in UI:**

- history is replay buffer only
- max 50 entries
- not a full audit log

#### Tab: Config

**Source:**

- `/_foundry/ws/channels`

**Show:**

- channel id
- presence enabled
- replay count
- allow client events
- requires auth
- guard
- permissions

This is the "why is this channel behaving this way?" tab.

### Widget: Idle Channels

**Source:**

- merged `/_foundry/ws/channels` + `/_foundry/ws/stats`

**Definition:**

- `active_subscriptions = 0`
- `inbound_messages_total = 0`
- `outbound_messages_total = 0`

**Show:**

- compact list of registered but inactive channels

This is valuable because Foundry pre-seeds registered channels into stats even when idle.

---

## WebSocket UX Rules

- Poll `/_foundry/ws/stats` and `/_foundry/ws/channels` every `5s`
- Poll `/_foundry/ws/presence/:channel` every `5s` only when the drawer presence tab is open
- Poll `/_foundry/ws/history/:channel` only when the history tab is open
- Preserve selected channel while polling
- Use soft refresh, not full page re-render

### Important product note

Foundry websocket counters are **per-process**, not globally aggregated across all app instances.

The UI should include a small note such as:

`Counters shown here are node-local unless your deployment routes all traffic through one process.`

If the project is single-node, the note can be quieter.

---

## Shared UI Components

Build both dashboards from a shared small admin kit:

- `StatusBadge`
- `MetricCard`
- `SectionCard`
- `KeyValueList`
- `DataTable`
- `JsonViewer`
- `RefreshIndicator`
- `EmptyState`
- `ErrorState`
- `RightDrawer`

This keeps the admin area consistent and avoids one-off widget code.

---

## Error / Empty States

### Jobs

- no failed jobs:
  - `No failed or retried jobs found`
- jobs history disabled or table missing:
  - `Job history is unavailable. Check Foundry jobs history configuration and migrations.`

### WebSocket

- unregistered channel:
  - `Channel not registered`
- presence not enabled:
  - `Presence is not enabled for this channel`
- history payload redacted:
  - `Payload hidden by server configuration`
- no channels:
  - `No WebSocket channels registered`

---

## Endpoint Mapping Summary

### Jobs Dashboard

| Widget | Endpoint | Fields |
| ------ | -------- | ------ |
| Runtime Health | `/_foundry/health`, `/_foundry/ready` | liveness state, readiness state, probes |
| Jobs KPIs | `/_foundry/runtime` | `jobs.*`, `scheduler.*`, `backend`, `bootstrap_complete` |
| Job Status Breakdown | `/_foundry/jobs/stats` | `stats[].status`, `stats[].count` |
| Recent Failed / Retried Jobs | `/_foundry/jobs/failed` | `failed_jobs[]` |

### WebSocket Dashboard

| Widget | Endpoint | Fields |
| ------ | -------- | ------ |
| Global WS KPIs | `/_foundry/ws/stats` | `global.*` |
| Top Active Channels | `/_foundry/ws/stats` | `channels[]` |
| Channel Registry | `/_foundry/ws/channels`, `/_foundry/ws/stats` | registry + counters merged by `id` |
| Presence | `/_foundry/ws/presence/:channel` | `count`, `members[]` |
| Recent History | `/_foundry/ws/history/:channel` | `messages[]` |

---

## Suggested Delivery Order

- [ ] Create `Admin > Observability > Jobs` page
- [ ] Build shared metric card / table / drawer primitives
- [ ] Wire jobs data sources and loading states
- [ ] Create `Admin > Observability > WebSocket` page
- [ ] Build merged channel registry + stats table
- [ ] Add channel detail drawer with `Stats`, `Presence`, `Recent History`, `Config` tabs
- [ ] Add polling and refresh indicators
- [ ] Add empty/error states
- [ ] Add access control and hide menu for non-admin users
- [ ] QA on desktop and mobile breakpoints

---

## Explicit Scope For The Agent

**Build now:**

- one Jobs dashboard page
- one WebSocket dashboard page
- shared admin widgets
- read-only access only

**Do not build now:**

- actions or mutations
- job retry/replay controls
- websocket disconnect controls
- per-room analytics
- global multi-node aggregation
- historical trend storage

---

## Final Recommendation

Yes: create **two dashboards**, not one mixed page.

Why:

- jobs and realtime are different operator workflows
- the datasets have different refresh rhythms
- websocket drilldowns want drawers/tabs
- jobs wants charts + failure table

The best v1 deliverable is:

- `Jobs Dashboard`: health, counters, status chart, recent failed jobs
- `WebSocket Dashboard`: counters, channel registry, presence, recent history

That is the highest-signal project-level admin UI Foundry can support today with the shipped endpoints.
